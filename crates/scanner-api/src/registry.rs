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

    /// Opens the first scanner there is.
    pub fn open_first(&self) -> Result<Box<dyn Scanner>> {
        let mut last = None;
        for device in self.list() {
            match self.open(&device) {
                Ok(scanner) => return Ok(scanner),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Disconnected("no scanner found".into())))
    }
}
