// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanners of every backend in one list.

use crate::{Capabilities, Error, Result, Scanner};

/// A scanner a backend found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The backend that found it ([`Backend::name`]).
    pub backend: &'static str,
    /// What the backend needs to open it again (a USB path, an address).
    pub id: String,
    /// What to show: manufacturer and model.
    pub name: String,
}

impl std::fmt::Display for DeviceInfo {
    /// What a list of scanners shows: the name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// A way of reaching scanners (USB for one chip family, the network, a
/// system library).
pub trait Backend: Send + Sync {
    /// Short stable name ("rts8822", "escl", "fake").
    fn name(&self) -> &'static str;

    /// The scanners there are now. A backend that cannot look (no
    /// permission, library missing) answers an empty list.
    fn list(&self) -> Vec<DeviceInfo>;

    /// Opens one of the scanners of [`Backend::list`].
    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Scanner>>;

    /// What the models this backend knows can do, connected or not: what
    /// the application shows before a scanner answers.
    fn models(&self) -> Vec<Capabilities> {
        Vec::new()
    }

    /// Why [`Backend::list`] finds no scanner, in the backend's own words,
    /// when it can tell without sending a scanner anything (a device it saw
    /// but could not open: in use by another program, access denied).
    /// `None` when it has nothing to say.
    fn why_none(&self) -> Option<String> {
        None
    }
}

/// Every backend the application knows, in order of preference.
#[derive(Default)]
pub struct Registry {
    backends: Vec<Box<dyn Backend>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.backends.iter().map(|b| b.name()))
            .finish()
    }
}

impl Registry {
    /// No backends yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a backend after the ones there are.
    pub fn with(mut self, backend: impl Backend + 'static) -> Self {
        self.backends.push(Box::new(backend));
        self
    }

    /// The scanners of every backend, backend by backend.
    pub fn list(&self) -> Vec<DeviceInfo> {
        self.backends.iter().flat_map(|b| b.list()).collect()
    }

    /// The models every backend knows, backend by backend.
    pub fn models(&self) -> Vec<Capabilities> {
        self.backends.iter().flat_map(|b| b.models()).collect()
    }

    /// Opens a scanner found by [`Registry::list`].
    pub fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Scanner>> {
        self.backends
            .iter()
            .find(|b| b.name() == device.backend)
            .ok_or_else(|| Error::Disconnected(format!("no backend {}", device.backend)))?
            .open(device)
    }

    /// Opens the first scanner there is. With none listed, the error says
    /// what the backends know of why ([`Backend::why_none`]); with none that
    /// opens, what each one answered, as the most telling of their errors
    /// (busy before not there).
    pub fn open_first(&self) -> Result<Box<dyn Scanner>> {
        let devices = self.list();
        if devices.is_empty() {
            let why: Vec<String> = self.backends.iter().filter_map(|b| b.why_none()).collect();
            return Err(Error::Disconnected(if why.is_empty() {
                "no scanner found".into()
            } else {
                format!("no scanner found: {}", why.join("; "))
            }));
        }
        let mut errors = Vec::new();
        for device in devices {
            match self.open(&device) {
                Ok(scanner) => return Ok(scanner),
                Err(e) => errors.push((device.name, e)),
            }
        }
        Err(most_telling(errors))
    }
}

/// One error for the scanners that would not open: the kind of the most
/// telling one (busy, then anything else, then not there), and what each
/// said when there were several.
fn most_telling(errors: Vec<(String, Error)>) -> Error {
    let rank = |e: &Error| match e {
        Error::Busy(_) => 0,
        Error::Other(_)
        | Error::InvalidRequest(_)
        | Error::Unsupported(_)
        | Error::FirmwareMissing(_) => 1,
        Error::Disconnected(_) => 2,
        Error::Cancelled => 3,
    };
    let Some(best) = errors
        .iter()
        .map(|(_, e)| e)
        .min_by_key(|e| rank(e))
        .cloned()
    else {
        return Error::Disconnected("no scanner found".into());
    };
    if errors.len() == 1 {
        return best;
    }
    let all = errors
        .iter()
        .map(|(name, e)| format!("{name}: {e}"))
        .collect::<Vec<_>>()
        .join("; ");
    match best {
        Error::Busy(_) => Error::Busy(all),
        Error::Other(_) => Error::Other(all),
        Error::InvalidRequest(_) => Error::InvalidRequest(all),
        Error::Disconnected(_) => Error::Disconnected(all),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_most_telling_error_keeps_what_each_said() {
        let e = most_telling(vec![
            ("A".into(), Error::Busy("Epson Scan is scanning".into())),
            ("B".into(), Error::Disconnected("gone".into())),
        ]);
        let Error::Busy(text) = e else {
            panic!("{e:?}")
        };
        assert!(text.contains("A: Epson Scan is scanning"), "{text}");
        assert!(text.contains("B: gone"), "{text}");
        let one = most_telling(vec![("A".into(), Error::Disconnected("gone".into()))]);
        assert_eq!(one, Error::Disconnected("gone".into()));
    }

    /// A backend that lists nothing and says why.
    struct Silent(Option<&'static str>);

    impl Backend for Silent {
        fn name(&self) -> &'static str {
            "silent"
        }
        fn list(&self) -> Vec<DeviceInfo> {
            Vec::new()
        }
        fn open(&self, _: &DeviceInfo) -> Result<Box<dyn Scanner>> {
            unreachable!("nothing listed")
        }
        fn why_none(&self) -> Option<String> {
            self.0.map(String::from)
        }
    }

    #[test]
    fn with_nothing_listed_the_backends_say_why() {
        let r = Registry::new()
            .with(Silent(None))
            .with(Silent(Some(r"\\.\Usbscan0: in use by another program")));
        let e = r.open_first().err().expect("nothing opens").to_string();
        assert!(e.contains("in use by another program"), "{e}");
        let e = Registry::new()
            .with(Silent(None))
            .open_first()
            .err()
            .unwrap();
        assert_eq!(e, Error::Disconnected("no scanner found".into()));
    }
}
