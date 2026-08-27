//! The seam between walgit and the operating system: every call whose shape differs
//! between Unix and Windows goes through here. Three things are actually different for
//! us — symlink creation, positioned writes and asking the filesystem how full it is —
//! so keeping them in one place keeps each OS branch visible and reviewed together.
//!
//! Portability notes baked into these contracts:
//! * **Symlinks** on Windows are real NTFS symlinks (`CreateSymbolicLinkW`): creating
//!   one needs Developer Mode or `SeCreateSymbolicLinkPrivilege`. Everything that links
//!   a store-mount base pack into a pack dir assumes that holds, same assumption a
//!   POSIX deployment makes about mounts.
//! * **Capacity** reports what an *unprivileged caller* can write (statvfs `f_bavail`,
//!   `GetDiskFreeSpaceExW`'s user free), which is what cache eviction decisions want.

use std::io;
use std::path::Path;

/// Create `link` pointing at `target`. The kind follows `target`: a directory target
/// gets a directory symlink, everything else a file symlink (the distinction matters
/// only on Windows, where they are different APIs).
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    match std::fs::metadata(target) {
        Ok(m) if m.is_dir() => symlink_dir(target, link),
        _ => symlink_file(target, link),
    }
}

/// `(free_to_caller, total)` bytes on the filesystem holding `path`; `None` when the
/// answer cannot be had (then callers degrade, e.g. eviction falls back to budgeting).
pub fn capacity(path: &Path) -> Option<(u64, u64)> {
    capacity_impl(path)
}

/// Write every byte of `buf` starting at absolute `offset`, in the spirit of
/// `pwrite`/`write_all_at`: the file's cursor stays untouched.
pub fn write_all_at(file: &std::fs::File, offset: u64, buf: &[u8]) -> io::Result<()> {
    write_all_at_impl(file, offset, buf)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(unix)]
fn capacity_impl(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    #[allow(unsafe_code)] // a zeroed-bytes struct the next call initializes
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a live NUL-terminated path buffer; `st` a valid statvfs slot
    // handed to statvfs which fills it or returns nonzero.
    #[allow(unsafe_code)]
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let total = st.f_blocks as u64 * st.f_frsize as u64;
    let avail = st.f_bavail as u64 * st.f_frsize as u64;
    Some((avail, total))
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(windows)]
fn capacity_impl(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    // NUL-terminated UTF-16: what GetDiskFreeSpaceExW takes instead of a CString.
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut caller_free: u64 = 0;
    let mut total: u64 = 0;
    let mut volume_free: u64 = 0;
    // SAFETY: `wide` is a live NUL-terminated UTF-16 buffer; the three out-params are
    // valid u64 slots for the callee to fill.
    #[allow(unsafe_code)] // one Win32 probe, reviewed at the platform seam
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut caller_free,
            &mut total,
            &mut volume_free,
        )
    };
    if ok == 0 {
        return None;
    }
    Some((caller_free, total))
}

#[cfg(unix)]
fn write_all_at_impl(file: &std::fs::File, offset: u64, buf: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_all_at_impl(file: &std::fs::File, mut offset: u64, mut buf: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    // `seek_write` writes up to one call's worth (short writes are legal); loop like
    // `WriteFile`'s overlapped users must. A zero-byte result is EOF-ish — fail loudly
    // rather than spin.
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "failed to write {}/{} bytes at offset {}",
                    buf.len(),
                    buf.len(),
                    offset
                ),
            ));
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}
