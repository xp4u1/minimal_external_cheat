//! Process handling: finding the target, reading and writing its memory.

use std::ffi::c_void;
use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32First, Module32Next, Process32First, Process32Next,
    MODULEENTRY32, PROCESSENTRY32, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION,
    PROCESS_VM_READ, PROCESS_VM_WRITE,
};

/// Access rights requested for the target process.
const ACCESS: u32 = PROCESS_VM_READ
    | PROCESS_VM_WRITE
    | PROCESS_VM_OPERATION
    | PROCESS_QUERY_INFORMATION
    | SYNCHRONIZE;

/// An attached game process together with the base address of its main module.
pub struct Process {
    handle: HANDLE,
    pub base: usize,
}

impl Process {
    /// Attaches to the process `name` (case-insensitive) and locates its main module.
    ///
    /// Returns `Ok(None)` if the process is not running or its module list is not available
    /// yet, in which case the caller should retry.
    ///
    /// # Errors
    ///
    /// Returns an error if the process exists but cannot be opened, typically because of
    /// missing privileges.
    pub fn try_attach(name: &str) -> io::Result<Option<Self>> {
        let Some(pid) = find_pid(name)? else {
            return Ok(None);
        };

        // SAFETY: plain FFI call; the result is checked for null below.
        let handle = unsafe { OpenProcess(ACCESS, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        // Owning the handle from here on closes it on every return path.
        let mut process = Self { handle, base: 0 };

        // The module list can be briefly unavailable while a process starts; retry later.
        match find_module_base(pid, name).ok().flatten() {
            Some(base) => {
                process.base = base;
                Ok(Some(process))
            }
            None => Ok(None),
        }
    }

    /// Returns true once the target process has terminated.
    pub fn exited(&self) -> bool {
        // SAFETY: the handle has SYNCHRONIZE access; a zero timeout never blocks.
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_OBJECT_0 }
    }

    /// Reads exactly `N` bytes from `addr` in the target.
    fn read<const N: usize>(&self, addr: usize) -> io::Result<[u8; N]> {
        let mut buf = [0u8; N];
        let mut done = 0usize;
        // SAFETY: `buf` is a valid writable buffer of `N` bytes for the duration of the call.
        let ok = unsafe {
            ReadProcessMemory(
                self.handle,
                addr as *const c_void,
                buf.as_mut_ptr().cast(),
                N,
                &mut done,
            )
        };
        check_transfer(ok, done, N)?;
        Ok(buf)
    }

    /// Reads a little-endian `i32` from `addr`.
    pub fn read_i32(&self, addr: usize) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.read(addr)?))
    }

    /// Reads a 64-bit pointer from `addr`.
    pub fn read_ptr(&self, addr: usize) -> io::Result<usize> {
        Ok(u64::from_le_bytes(self.read(addr)?) as usize)
    }

    /// Writes a little-endian `i32` to `addr`.
    pub fn write_i32(&self, addr: usize, value: i32) -> io::Result<()> {
        let bytes = value.to_le_bytes();
        let mut done = 0usize;
        // SAFETY: `bytes` is a valid readable buffer for the duration of the call.
        let ok = unsafe {
            WriteProcessMemory(
                self.handle,
                addr as *const c_void,
                bytes.as_ptr().cast(),
                bytes.len(),
                &mut done,
            )
        };
        check_transfer(ok, done, bytes.len())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle was opened in `try_attach` and is closed only here.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

/// Maps a `BOOL` result plus a transferred byte count onto an `io::Result`.
fn check_transfer(ok: i32, transferred: usize, expected: usize) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else if transferred != expected {
        Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short memory transfer"))
    } else {
        Ok(())
    }
}

/// Returns true if the NUL-terminated ToolHelp name `buf` equals `name` (ASCII case-insensitive).
fn name_eq(buf: &[i8], name: &str) -> bool {
    buf.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| (c as u8).to_ascii_lowercase())
        .eq(name.bytes().map(|b| b.to_ascii_lowercase()))
}

/// RAII owner of a ToolHelp snapshot handle.
struct Snapshot(HANDLE);

impl Snapshot {
    fn new(flags: u32, pid: u32) -> io::Result<Self> {
        // SAFETY: plain FFI call without pointer arguments.
        let handle = unsafe { CreateToolhelp32Snapshot(flags, pid) };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateToolhelp32Snapshot and is closed only here.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Finds the PID of the first process whose executable name equals `name`.
fn find_pid(name: &str) -> io::Result<Option<u32>> {
    let snapshot = Snapshot::new(TH32CS_SNAPPROCESS, 0)?;

    // SAFETY: `PROCESSENTRY32` is a plain C struct for which all-zero bytes are valid.
    let mut entry: PROCESSENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;

    // SAFETY: the snapshot is valid and `entry` is a correctly sized, writable struct.
    let mut ok = unsafe { Process32First(snapshot.0, &mut entry) };
    while ok != 0 {
        if name_eq(&entry.szExeFile, name) {
            return Ok(Some(entry.th32ProcessID));
        }
        // SAFETY: same invariants as `Process32First` above.
        ok = unsafe { Process32Next(snapshot.0, &mut entry) };
    }
    Ok(None)
}

/// Finds the base address of the module `name` inside process `pid`.
fn find_module_base(pid: u32, name: &str) -> io::Result<Option<usize>> {
    let snapshot = Snapshot::new(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)?;

    // SAFETY: `MODULEENTRY32` is a plain C struct for which all-zero bytes are valid.
    let mut entry: MODULEENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<MODULEENTRY32>() as u32;

    // SAFETY: the snapshot is valid and `entry` is a correctly sized, writable struct.
    let mut ok = unsafe { Module32First(snapshot.0, &mut entry) };
    while ok != 0 {
        if name_eq(&entry.szModule, name) {
            return Ok(Some(entry.modBaseAddr as usize));
        }
        // SAFETY: same invariants as `Module32First` above.
        ok = unsafe { Module32Next(snapshot.0, &mut entry) };
    }
    Ok(None)
}
