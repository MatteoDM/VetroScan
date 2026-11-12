// SPDX-License-Identifier: GPL-2.0-or-later
//! The scanner through `usbscan.sys`, the Windows still-image driver that
//! Epson's driver package (`es54.inf`) binds to it: no driver change, and
//! Epson's software keeps working.
//!
//! The device shows up as `\\.\UsbscanN`. Epson's own WIA driver talks to
//! it only with `ReadFile` / `WriteFile` (bulk in and out), and so does
//! this transport; `DeviceIoControl` is used only to read the USB ids and
//! set the timeouts. Structures and IOCTL codes from `usbscan.h` (Windows
//! DDK).
//!
//! While it is open the transport holds the scanner's STI lock
//! ([`crate::sti`]), so Windows' image service does not poll the scanner in
//! between.
#![allow(unsafe_code)]

use crate::sti::StiLock;
use crate::{Found, Link, Transport, BULK_TIMEOUT, PRODUCT_ID, VENDOR_ID};
use scanner_api::{Error, Result};
use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// Highest `\\.\UsbscanN` index probed.
const MAX_DEVICES: u32 = 32;

const fn ctl_code(function: u32) -> u32 {
    // CTL_CODE(FILE_DEVICE_USB_SCAN, IOCTL_INDEX + n, METHOD_BUFFERED, FILE_ANY_ACCESS)
    (0x8000 << 16) | ((0x0800 + function) << 2)
}

const IOCTL_GET_DEVICE_DESCRIPTOR: u32 = ctl_code(6);
const IOCTL_SET_TIMEOUT: u32 = ctl_code(11);

/// `DEVICE_DESCRIPTOR` as returned by `IOCTL_GET_DEVICE_DESCRIPTOR`.
#[repr(C)]
#[derive(Default)]
struct DeviceDescriptor {
    vendor_id: u16,
    product_id: u16,
    bcd_device: u16,
    language_id: u16,
}

/// `USBSCAN_TIMEOUT`, in seconds.
#[repr(C)]
struct UsbscanTimeout {
    read: u32,
    write: u32,
    event: u32,
}

fn last_error(context: &str) -> Error {
    // SAFETY: GetLastError has no preconditions.
    let code = unsafe { GetLastError() };
    Error::Disconnected(format!("{context}: Windows error {code}"))
}

fn path_of(n: u32) -> String {
    format!(r"\\.\Usbscan{n}")
}

/// An owned Win32 handle.
struct Handle(HANDLE);

impl Handle {
    fn open(n: u32) -> Option<Self> {
        let path: Vec<u16> = path_of(n).encode_utf16().chain(Some(0)).collect();
        // SAFETY: `path` is NUL-terminated and outlives the call.
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(Self(h))
    }

    /// Synchronous `DeviceIoControl`; the bytes written to `output`.
    ///
    /// # Safety
    /// `input` / `output` must be valid for their lengths.
    unsafe fn ioctl(
        &self,
        code: u32,
        input: *const c_void,
        in_len: u32,
        output: *mut c_void,
        out_len: u32,
    ) -> Option<u32> {
        let mut returned = 0u32;
        let ok = DeviceIoControl(
            self.0,
            code,
            input,
            in_len,
            output,
            out_len,
            &mut returned,
            null_mut(),
        );
        (ok != 0).then_some(returned)
    }

    /// [`BULK_TIMEOUT`] for reads and writes. Failure is not fatal: the driver's default
    /// timeout then applies.
    fn set_timeout(&self) {
        let secs = BULK_TIMEOUT.as_secs() as u32;
        let timeout = UsbscanTimeout {
            read: secs,
            write: secs,
            event: 0,
        };
        // SAFETY: `timeout` is a valid input buffer of the given size.
        let _ = unsafe {
            self.ioctl(
                IOCTL_SET_TIMEOUT,
                (&timeout as *const UsbscanTimeout).cast(),
                size_of::<UsbscanTimeout>() as u32,
                null_mut(),
                0,
            )
        };
    }

    /// `ReadFile` on the bulk IN pipe: the bytes read.
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let len = u32::try_from(buf.len())
            .map_err(|_| Error::InvalidRequest("bulk read too long".into()))?;
        let mut read = 0u32;
        // SAFETY: `buf` is valid for `len` bytes; synchronous handle.
        let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr(), len, &mut read, null_mut()) };
        if ok == 0 {
            return Err(last_error("bulk IN"));
        }
        Ok(read as usize)
    }

    /// `WriteFile` on the bulk OUT pipe, all of `data`.
    fn write(&self, data: &[u8]) -> Result<()> {
        let len = u32::try_from(data.len())
            .map_err(|_| Error::InvalidRequest("bulk write too long".into()))?;
        let mut written = 0u32;
        // SAFETY: `data` is valid for `len` bytes; synchronous handle.
        let ok = unsafe { WriteFile(self.0, data.as_ptr(), len, &mut written, null_mut()) };
        if ok == 0 {
            return Err(last_error("bulk OUT"));
        }
        if written != len {
            return Err(Error::Other(format!(
                "bulk OUT: {written} bytes of {len} sent"
            )));
        }
        Ok(())
    }

    fn is_4490(&self) -> bool {
        let mut d = DeviceDescriptor::default();
        // SAFETY: `d` is a valid output buffer of the given size.
        let n = unsafe {
            self.ioctl(
                IOCTL_GET_DEVICE_DESCRIPTOR,
                null(),
                0,
                (&mut d as *mut DeviceDescriptor).cast(),
                size_of::<DeviceDescriptor>() as u32,
            )
        };
        n.is_some_and(|n| n as usize >= size_of::<DeviceDescriptor>())
            && d.vendor_id == VENDOR_ID
            && d.product_id == PRODUCT_ID
    }
}

// SAFETY: a Win32 file handle may be used from any thread; each `Handle` is owned and closed
// once. A synchronous handle serialises its own I/O, so a read and a write at once need two
// handles ([`UsbscanTransport::write_reading`]).
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// The scanners reachable through usbscan.sys.
pub(crate) fn list() -> Vec<Found> {
    (0..MAX_DEVICES)
        .filter(|&n| Handle::open(n).is_some_and(|h| h.is_4490()))
        .map(|n| Found {
            link: Link::Usbscan,
            location: path_of(n),
            note: None,
        })
        .collect()
}

/// The scanner, opened through usbscan.sys.
pub struct UsbscanTransport {
    handle: Handle,
    index: u32,
    /// Released after the handle is closed (fields drop in order).
    lock: Option<StiLock>,
}

// SAFETY: the handle is owned by the transport and closed once; Win32 file
// handles may be used from any thread, and the transport is only used
// through `&mut self`. No I/O is left pending (all of it is synchronous).
unsafe impl Send for UsbscanTransport {}

impl fmt::Debug for UsbscanTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UsbscanTransport")
            .field("path", &path_of(self.index))
            .field("lock", &self.lock)
            .finish_non_exhaustive()
    }
}

/// How long opening waits for the STI lock that Windows does not give yet.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

impl UsbscanTransport {
    /// Opens `\\.\UsbscanN` (`location`, as [`list`] gives it), if it is
    /// the Epson 4490, and takes its STI lock; a lock another program holds
    /// (Epson Scan scanning) is an [`Error::Busy`].
    pub fn open(location: &str) -> Result<Self> {
        let mut t = Self::open_unlocked(location)?;
        t.hold()?;
        Ok(t)
    }

    /// Opens `\\.\UsbscanN` without the STI lock, only to read what a scanner left half-way
    /// through a scan still sends (`epson4490 drain --no-lock`). Taking the lock has Epson's
    /// driver ask the scanner its state (`1B 03`); a scanner still sending its image answers
    /// with image bytes, which Epson's driver took for "no firmware": it began an upload
    /// (`1B 06`), which the scanner did not take (capture 44, 12 November 2026).
    pub fn open_unlocked(location: &str) -> Result<Self> {
        let n: u32 = location
            .strip_prefix(r"\\.\Usbscan")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| Error::Disconnected(format!("not a usbscan device: {location}")))?;
        let handle = Handle::open(n).ok_or_else(|| last_error("opening usbscan device"))?;
        if !handle.is_4490() {
            return Err(Error::Disconnected(format!(
                "{location} is not an Epson Perfection 4490"
            )));
        }
        handle.set_timeout();
        Ok(Self {
            handle,
            index: n,
            lock: None,
        })
    }

    /// Whether the transport holds the scanner's STI lock.
    pub fn locked(&self) -> bool {
        self.lock.is_some()
    }
}

impl Transport for UsbscanTransport {
    /// Takes the STI lock, waiting up to [`LOCK_WAIT`] for Windows to have one to give; a
    /// lock another program holds is an [`Error::Busy`] at once.
    fn hold(&mut self) -> Result<()> {
        if self.lock.is_some() {
            return Ok(());
        }
        // Without the lock Windows' image service asks the scanner for its buttons between
        // our commands and the answers mix: never used so. Right after the scanner is
        // switched on (12 November 2026) the lock could not be had for a while: tried again.
        let until = std::time::Instant::now() + LOCK_WAIT;
        let lock = loop {
            if let Some(lock) = StiLock::take(&path_of(self.index))? {
                break lock;
            }
            if std::time::Instant::now() > until {
                return Err(Error::Busy(format!(
                    "Windows' image service did not let go of the Epson 4490 in {} s (switched \
                     on a moment ago, or not answering): try again in a little while, or switch \
                     it off and on",
                    LOCK_WAIT.as_secs()
                )));
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        self.lock = Some(lock);
        Ok(())
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.handle.read(buf)
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        self.handle.write(data)
    }

    /// A second handle of the same device reads on its own thread while this one writes (a
    /// synchronous handle would do one after the other). Tried on the scanner on 12 November
    /// 2026 (capture 53): a scanner left half-way, whose `1E 18` alone had waited 30 s and been
    /// given up (capture 52), took it at once.
    fn write_reading(
        &mut self,
        data: &[u8],
        on_read: &mut (dyn FnMut(&[u8]) -> bool + Send),
    ) -> Result<()> {
        let reader =
            Handle::open(self.index).ok_or_else(|| last_error("opening usbscan device"))?;
        reader.set_timeout();
        let writer = &self.handle;
        std::thread::scope(|s| {
            let read = s.spawn(move || -> Result<()> {
                let mut buf = vec![0u8; crate::WRITE_READING_CHUNK];
                loop {
                    let n = reader.read(&mut buf)?;
                    if !on_read(&buf[..n]) {
                        return Ok(());
                    }
                }
            });
            // The read is asked first; then the write.
            std::thread::sleep(std::time::Duration::from_millis(100));
            let written = writer.write(data);
            let read = read
                .join()
                .unwrap_or_else(|_| Err(Error::Other("bulk IN: the reader stopped".into())));
            written.and(read)
        })
    }
}
