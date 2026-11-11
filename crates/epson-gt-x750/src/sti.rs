// SPDX-License-Identifier: GPL-2.0-or-later
//! The Still Image (STI) lock of the scanner, on Windows with Epson's driver.
//!
//! Epson's driver declares that the scanner needs polling for its buttons
//! (`Capabilities` 0x13 in its device key), so Windows' image service
//! (`stisvc`) asks it `1E 85` every second through the same `usbscan.sys`
//! device this backend writes to: its exchanges could fall between a command
//! of ours and its answer. While a program holds the device's STI lock the
//! service does not poll it (captured on 11 November 2026: no `1E 85` for as
//! long as the lock was held, polling again at once after it). The backend
//! takes the lock for as long as it has the scanner open, and reads the
//! buttons itself.
//!
//! `StiCreateInstanceW` is loaded from `sti.dll` at run time, and the two
//! COM interfaces are called through their tables (`sti.h`); the device's
//! STI name is the `DeviceID` of the image-class key whose `CreateFileName`
//! is the `\\.\UsbscanN` opened. When any of it is missing the backend goes
//! on without the lock, as before.
#![allow(unsafe_code)]

use scanner_api::{Error, Result};
use std::ffi::c_void;
use std::fmt;
use std::ptr::{null, null_mut};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

/// The image class, under which Windows keeps each scanner's driver key.
const IMAGE_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{6BDD1FC6-810F-11D0-BEC7-08002BE2092F}";
/// How many of its keys (`0000`, `0001`, …) are looked at.
const MAX_KEYS: u32 = 100;
/// `STI_VERSION_REAL | STI_VERSION_FLAG_UNICODE`.
const STI_VERSION: u32 = 0x0000_0002 | 0x0100_0000;
/// `STI_DEVICE_CREATE_STATUS`: the device is not opened for data.
const CREATE_STATUS: u32 = 1;
/// How long to wait for a lock another program holds (Epson Scan scanning).
const LOCK_TIMEOUT_MS: u32 = 5000;

// Places in the interfaces' tables (`sti.h`): IUnknown's three, then
// IStillImageW::Initialize, GetDeviceList, GetDeviceInfo, CreateDevice;
// IStiDevice::Initialize, GetCapabilities, GetStatus, DeviceReset,
// Diagnostic, Escape, GetLastError, LockDevice, UnLockDevice.
const RELEASE: usize = 2;
const CREATE_DEVICE: usize = 6;
const LOCK_DEVICE: usize = 10;
const UNLOCK_DEVICE: usize = 11;

type StiCreateInstanceW =
    unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut c_void) -> i32;
type CreateDevice =
    unsafe extern "system" fn(*mut c_void, *const u16, u32, *mut *mut c_void, *mut c_void) -> i32;
type LockDevice = unsafe extern "system" fn(*mut c_void, u32) -> i32;
type NoArgs = unsafe extern "system" fn(*mut c_void) -> i32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A string value of `HKLM\<key>`.
fn reg_string(key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the strings are NUL-terminated; `buf` is valid for `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let chars = (size as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..chars]);
    Some(s.trim_end_matches('\0').to_string())
}

/// The STI name (`{6BDD1FC6-…}\NNNN`) of the scanner opened as `port`
/// (`\\.\UsbscanN`).
fn sti_name(port: &str) -> Option<String> {
    (0..MAX_KEYS).find_map(|i| {
        let key = format!(r"{IMAGE_CLASS}\{i:04}");
        reg_string(&key, "CreateFileName")
            .filter(|p| p.eq_ignore_ascii_case(port))
            .and_then(|_| reg_string(&key, "DeviceID"))
    })
}

/// The method at `place` of the COM object `obj`.
///
/// # Safety
/// `obj` must be a live COM object whose table has more than `place`
/// entries, and `F` the method's real type.
unsafe fn method<F: Copy>(obj: *mut c_void, place: usize) -> F {
    let table = *(obj as *const *const usize);
    let entry = table.add(place);
    std::mem::transmute_copy(&*entry)
}

/// Releases a COM object.
///
/// # Safety
/// `obj` must be a live COM object this code holds a reference to.
unsafe fn release(obj: *mut c_void) {
    let f: ReleaseFn = method(obj, RELEASE);
    f(obj);
}

/// The scanner's STI lock, held until dropped.
pub(crate) struct StiLock {
    sti: *mut c_void,
    device: *mut c_void,
    name: String,
}

// SAFETY: the two objects are owned by the lock and released once; STI's
// in-process objects are not tied to the thread that made them (they only
// talk to the image service), and the lock is never shared.
unsafe impl Send for StiLock {}

impl fmt::Debug for StiLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StiLock").field("name", &self.name).finish()
    }
}

impl StiLock {
    /// Takes the lock of the scanner opened as `port`. `Ok(None)` when it
    /// has none to take (no STI device for it, or STI missing); an error
    /// when another program holds it.
    pub(crate) fn take(port: &str) -> Result<Option<Self>> {
        let Some(name) = sti_name(port) else {
            return Ok(None);
        };
        let dll = wide("sti.dll");
        // SAFETY: `dll` is NUL-terminated; the library is kept loaded.
        let lib = unsafe { LoadLibraryW(dll.as_ptr()) };
        if lib.is_null() {
            return Ok(None);
        }
        // SAFETY: the name is a NUL-terminated ANSI string.
        let Some(create) = (unsafe { GetProcAddress(lib, c"StiCreateInstanceW".as_ptr().cast()) })
        else {
            return Ok(None);
        };
        // SAFETY: the export has this type (`sti.h`).
        let create: StiCreateInstanceW = unsafe { std::mem::transmute(create) };
        let mut sti = null_mut();
        // SAFETY: the module handle of this program, an out pointer, no outer object.
        let hr = unsafe { create(GetModuleHandleW(null()), STI_VERSION, &mut sti, null_mut()) };
        if hr < 0 || sti.is_null() {
            return Ok(None);
        }
        let wname = wide(&name);
        let mut device = null_mut();
        // SAFETY: `sti` is a live IStillImageW; `wname` is NUL-terminated.
        let hr = unsafe {
            let f: CreateDevice = method(sti, CREATE_DEVICE);
            f(sti, wname.as_ptr(), CREATE_STATUS, &mut device, null_mut())
        };
        if hr < 0 || device.is_null() {
            // SAFETY: `sti` is held here and released once.
            unsafe { release(sti) };
            return Ok(None);
        }
        // SAFETY: `device` is a live IStiDevice.
        let hr = unsafe {
            let f: LockDevice = method(device, LOCK_DEVICE);
            f(device, LOCK_TIMEOUT_MS)
        };
        if hr < 0 {
            // SAFETY: both are held here and released once.
            unsafe {
                release(device);
                release(sti);
            }
            return Err(Error::Busy(format!(
                "the Epson 4490 is in use by another program (its Windows lock was not \
                 free after {} s; error {hr:#010X})",
                LOCK_TIMEOUT_MS / 1000
            )));
        }
        Ok(Some(Self { sti, device, name }))
    }
}

impl Drop for StiLock {
    fn drop(&mut self) {
        // SAFETY: both objects are live and owned; unlocked and released once.
        unsafe {
            let unlock: NoArgs = method(self.device, UNLOCK_DEVICE);
            unlock(self.device);
            release(self.device);
            release(self.sti);
        }
    }
}
