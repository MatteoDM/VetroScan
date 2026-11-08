// SPDX-License-Identifier: GPL-2.0-or-later
//! Windows backend over `usbscan.sys`, the still-image USB driver that the HP
//! driver package binds to the scanner (`hpgt3770.inf` uses `sti.inf`).
//!
//! The device shows up as `\\.\UsbscanN`. Control requests go through
//! `DeviceIoControl`, bulk transfers through `ReadFile` / `WriteFile`, and
//! button events through `IOCTL_WAIT_ON_DEVICE_EVENT`. This is the same path
//! the HP driver itself uses, so nothing has to be reinstalled.
//!
//! Structures and IOCTL codes come from `usbscan.h` (Windows DDK).
#![allow(unsafe_code)]

use crate::{Backend, Found, BULK_TIMEOUT};
use hp3770_core::device::{PRODUCT_ID, VENDOR_ID};
use hp3770_core::protocol::{SetupPacket, VENDOR_REQUEST};
use hp3770_core::transport::Transport;
use hp3770_core::{Error, Result};
use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::{
    CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED,
};

/// Highest `\\.\UsbscanN` index probed.
const MAX_DEVICES: u32 = 32;

const fn ctl_code(function: u32) -> u32 {
    // CTL_CODE(FILE_DEVICE_USB_SCAN, IOCTL_INDEX + n, METHOD_BUFFERED, FILE_ANY_ACCESS)
    (0x8000 << 16) | ((0x0800 + function) << 2)
}

const IOCTL_WAIT_ON_DEVICE_EVENT: u32 = ctl_code(2);
const IOCTL_READ_REGISTERS: u32 = ctl_code(3);
const IOCTL_WRITE_REGISTERS: u32 = ctl_code(4);
const IOCTL_GET_DEVICE_DESCRIPTOR: u32 = ctl_code(6);
const IOCTL_SEND_USB_REQUEST: u32 = ctl_code(9);
const IOCTL_SET_TIMEOUT: u32 = ctl_code(11);

/// `IO_BLOCK` (usbscan.h, packed to 8).
#[repr(C)]
struct IoBlock {
    offset: u32,
    length: u32,
    data: *mut u8,
    index: u32,
}

/// `IO_BLOCK_EX` (usbscan.h, packed to 8).
#[repr(C)]
struct IoBlockEx {
    offset: u32,
    length: u32,
    data: *mut u8,
    index: u32,
    request: u8,
    request_type: u8,
    direction_in: u8,
}

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
    Error::Transport(format!("{context}: Windows error {code}"))
}

fn device_path(n: u32) -> Vec<u16> {
    format!(r"\\.\Usbscan{n}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

/// An owned Win32 handle.
struct Handle(HANDLE);

impl Handle {
    fn open(n: u32, overlapped: bool) -> Option<Self> {
        let path = device_path(n);
        let flags = if overlapped {
            FILE_FLAG_OVERLAPPED
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        // SAFETY: `path` is NUL-terminated and outlives the call.
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                flags,
                null_mut(),
            )
        };
        (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(Self(h))
    }

    /// Synchronous `DeviceIoControl`. Returns the bytes written to `output`.
    ///
    /// # Safety
    /// `input` / `output` must be valid for their lengths; any pointer inside
    /// `input` must stay valid for the call.
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

    fn descriptor(&self) -> Option<DeviceDescriptor> {
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
        }?;
        (n as usize >= size_of::<DeviceDescriptor>()).then_some(d)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed once.
        unsafe { CloseHandle(self.0) };
    }
}

fn is_3770(h: &Handle) -> bool {
    h.descriptor()
        .is_some_and(|d| d.vendor_id == VENDOR_ID && d.product_id == PRODUCT_ID)
}

/// Lists the scanners reachable through usbscan.sys.
pub(crate) fn list() -> Vec<Found> {
    (0..MAX_DEVICES)
        .filter(|&n| Handle::open(n, false).is_some_and(|h| is_3770(&h)))
        .map(|n| Found {
            backend: Backend::Usbscan,
            location: format!(r"\\.\Usbscan{n}"),
            note: None,
        })
        .collect()
}

/// A pending `IOCTL_WAIT_ON_DEVICE_EVENT`. Boxed so that the OVERLAPPED and
/// the buffer do not move while the kernel owns them.
struct PendingEvent {
    overlapped: OVERLAPPED,
    buffer: [u8; 8],
}

/// A scanner opened through usbscan.sys.
pub struct UsbscanScanner {
    sync: Handle,
    events: Handle,
    event_signal: Handle,
    pending: Option<Box<PendingEvent>>,
    index: u32,
}

// SAFETY: every handle is owned by the scanner and closed once, and the
// buffer of a pending overlapped read is boxed, so it stays where the
// kernel writes it when the scanner moves. Win32 file and event handles may
// be used from any thread; the scanner is only used through `&mut self`.
unsafe impl Send for UsbscanScanner {}

impl fmt::Debug for UsbscanScanner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UsbscanScanner")
            .field("path", &self.path())
            .finish_non_exhaustive()
    }
}

impl UsbscanScanner {
    /// Opens the first `\\.\UsbscanN` that is an HP Scanjet 3770.
    pub fn open_first() -> Result<Self> {
        (0..MAX_DEVICES)
            .find_map(|n| Self::open(n).ok())
            .ok_or_else(|| {
                Error::Transport("no \\\\.\\UsbscanN device is an HP Scanjet 3770".into())
            })
    }

    /// Opens `\\.\Usbscan{n}`.
    pub fn open(n: u32) -> Result<Self> {
        let sync = Handle::open(n, false).ok_or_else(|| last_error("opening usbscan device"))?;
        if !is_3770(&sync) {
            return Err(Error::Transport(format!(
                r"\\.\Usbscan{n} is not an HP Scanjet 3770"
            )));
        }
        let events =
            Handle::open(n, true).ok_or_else(|| last_error("opening usbscan event handle"))?;
        // SAFETY: manual-reset, initially non-signalled, unnamed event.
        let signal = unsafe { CreateEventW(null(), 1, 0, null()) };
        if signal.is_null() {
            return Err(last_error("creating event"));
        }
        let scanner = Self {
            sync,
            events,
            event_signal: Handle(signal),
            pending: None,
            index: n,
        };
        let secs = BULK_TIMEOUT.as_secs() as u32;
        let timeout = UsbscanTimeout {
            read: secs,
            write: secs,
            event: 0,
        };
        // SAFETY: `timeout` is a valid input buffer of the given size.
        // Failure is not fatal: the driver default timeout then applies.
        let _ = unsafe {
            scanner.sync.ioctl(
                IOCTL_SET_TIMEOUT,
                (&timeout as *const UsbscanTimeout).cast(),
                size_of::<UsbscanTimeout>() as u32,
                null_mut(),
                0,
            )
        };
        Ok(scanner)
    }

    /// The device path, `\\.\UsbscanN`.
    pub fn path(&self) -> String {
        format!(r"\\.\Usbscan{}", self.index)
    }

    /// Register requests (`bRequest 0x04`, vendor, more than one byte) use
    /// the dedicated IOCTLs, exactly like the HP driver.
    fn is_register_request(setup: SetupPacket) -> bool {
        setup.is_vendor()
            && setup.request == VENDOR_REQUEST
            && setup.length > 1
            && setup.request_type & 0x1f == 0
    }
}

impl Transport for UsbscanScanner {
    fn control_in(&mut self, setup: SetupPacket) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; usize::from(setup.length)];
        let len = u32::from(setup.length);
        // SAFETY: `buf` is valid for `len` bytes and outlives the call; the
        // IO block lives on the stack for the duration of the call.
        let n = unsafe {
            if Self::is_register_request(setup) {
                let block = IoBlock {
                    offset: setup.value.into(),
                    length: len,
                    data: buf.as_mut_ptr(),
                    index: setup.index.into(),
                };
                self.sync.ioctl(
                    IOCTL_READ_REGISTERS,
                    (&block as *const IoBlock).cast(),
                    size_of::<IoBlock>() as u32,
                    buf.as_mut_ptr().cast(),
                    len,
                )
            } else {
                let block = IoBlockEx {
                    offset: setup.value.into(),
                    length: len,
                    data: buf.as_mut_ptr(),
                    index: setup.index.into(),
                    request: setup.request,
                    request_type: setup.request_type,
                    direction_in: 1,
                };
                self.sync.ioctl(
                    IOCTL_SEND_USB_REQUEST,
                    (&block as *const IoBlockEx).cast(),
                    size_of::<IoBlockEx>() as u32,
                    buf.as_mut_ptr().cast(),
                    len,
                )
            }
        }
        .ok_or_else(|| last_error("control IN"))?;
        // Some usbscan versions report 0 bytes returned for register reads
        // even though the buffer was filled.
        if n != 0 {
            buf.truncate(n as usize);
        }
        Ok(buf)
    }

    fn control_out(&mut self, setup: SetupPacket, data: &[u8]) -> Result<()> {
        let mut buf = data.to_vec();
        let len = u32::try_from(buf.len())
            .map_err(|_| Error::InvalidArgument("control data too long".into()))?;
        // SAFETY: `buf` is valid for `len` bytes and outlives the call.
        unsafe {
            if Self::is_register_request(setup) {
                let block = IoBlock {
                    offset: setup.value.into(),
                    length: len,
                    data: buf.as_mut_ptr(),
                    index: setup.index.into(),
                };
                self.sync.ioctl(
                    IOCTL_WRITE_REGISTERS,
                    (&block as *const IoBlock).cast(),
                    size_of::<IoBlock>() as u32,
                    null_mut(),
                    0,
                )
            } else {
                let block = IoBlockEx {
                    offset: setup.value.into(),
                    length: len,
                    data: if buf.is_empty() {
                        null_mut()
                    } else {
                        buf.as_mut_ptr()
                    },
                    index: setup.index.into(),
                    request: setup.request,
                    request_type: setup.request_type,
                    direction_in: 0,
                };
                self.sync.ioctl(
                    IOCTL_SEND_USB_REQUEST,
                    (&block as *const IoBlockEx).cast(),
                    size_of::<IoBlockEx>() as u32,
                    null_mut(),
                    0,
                )
            }
        }
        .map(|_| ())
        .ok_or_else(|| last_error("control OUT"))
    }

    fn bulk_in(&mut self, buf: &mut [u8]) -> Result<usize> {
        let len = u32::try_from(buf.len())
            .map_err(|_| Error::InvalidArgument("bulk read too long".into()))?;
        let mut read = 0u32;
        // SAFETY: `buf` is valid for `len` bytes; synchronous handle.
        let ok = unsafe { ReadFile(self.sync.0, buf.as_mut_ptr(), len, &mut read, null_mut()) };
        if ok == 0 {
            return Err(last_error("bulk IN"));
        }
        Ok(read as usize)
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        let len = u32::try_from(data.len())
            .map_err(|_| Error::InvalidArgument("bulk write too long".into()))?;
        let mut written = 0u32;
        // SAFETY: `data` is valid for `len` bytes; synchronous handle.
        let ok = unsafe { WriteFile(self.sync.0, data.as_ptr(), len, &mut written, null_mut()) };
        if ok == 0 {
            return Err(last_error("bulk OUT"));
        }
        if written != len {
            return Err(Error::ShortTransfer {
                expected: data.len(),
                actual: written as usize,
            });
        }
        Ok(())
    }

    fn interrupt_in(&mut self, timeout: Duration) -> Result<Option<u8>> {
        if self.pending.is_none() {
            let mut p = Box::new(PendingEvent {
                // SAFETY: OVERLAPPED is plain data; all-zero is its initial state.
                overlapped: unsafe { std::mem::zeroed() },
                buffer: [0; 8],
            });
            p.overlapped.hEvent = self.event_signal.0;
            // SAFETY: the boxed OVERLAPPED and buffer stay at the same address
            // until the request completes or is cancelled in Drop.
            let ok = unsafe {
                DeviceIoControl(
                    self.events.0,
                    IOCTL_WAIT_ON_DEVICE_EVENT,
                    null(),
                    0,
                    p.buffer.as_mut_ptr().cast(),
                    p.buffer.len() as u32,
                    null_mut(),
                    &mut p.overlapped,
                )
            };
            // SAFETY: GetLastError has no preconditions.
            if ok == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(last_error("waiting for a button"));
            }
            self.pending = Some(p);
        }
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: valid event handle.
        match unsafe { WaitForSingleObject(self.event_signal.0, ms) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(None),
            _ => return Err(last_error("waiting for a button")),
        }
        let mut p = self.pending.take().expect("pending event");
        let mut n = 0u32;
        // SAFETY: the operation has completed (event signalled).
        let ok = unsafe { GetOverlappedResult(self.events.0, &p.overlapped, &mut n, 0) };
        if ok == 0 {
            return Err(last_error("button event"));
        }
        p.overlapped.hEvent = null_mut();
        Ok((n > 0).then_some(p.buffer[0]))
    }
}

impl Drop for UsbscanScanner {
    fn drop(&mut self) {
        if let Some(p) = self.pending.take() {
            let mut n = 0u32;
            // SAFETY: cancel the pending request and wait for the kernel to
            // release the boxed buffer before it is freed.
            unsafe {
                CancelIoEx(self.events.0, &p.overlapped);
                GetOverlappedResult(self.events.0, &p.overlapped, &mut n, 1);
            }
        }
    }
}
