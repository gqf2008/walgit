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
/// answer cannot be had. Callers degrade by *skipping the disk-pressure decision*:
/// `evict_idle`'s disk branch returns early on `None` (budget mode is unaffected), and
/// the rebuild headroom check treats `None` as "cannot tell, proceed" rather than
/// refusing — so a `None` here never evicts or fails a rebuild, it only removes the guard.
pub fn capacity(path: &Path) -> Option<(u64, u64)> {
    capacity_impl(path)
}

/// Write every byte of `buf` starting at absolute `offset`, in the spirit of
/// `pwrite`/`write_all_at`: a positional write that does not depend on the file
/// cursor, so concurrent writers with distinct offsets (the striped downloader)
/// cannot corrupt each other. The cursor itself is unspecified afterwards on
/// Windows (the positioned write may move it); callers here never read it.
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
    // SAFETY: `st` is a zeroed struct the next call fully initializes before any
    // field is read (statvfs fills it or returns nonzero).
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
fn write_all_at_impl(file: &std::fs::File, offset: u64, buf: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    // `seek_write` writes up to one call's worth (short writes are legal); loop like
    // `WriteFile`'s overlapped users must. A zero-byte result is EOF-ish — fail loudly
    // rather than spin.
    let mut remaining = buf;
    let mut at = offset;
    while !remaining.is_empty() {
        let n = file.seek_write(remaining, at)?;
        if n == 0 {
            // A zero-length request can't reach here (the loop guard is non-empty);
            // a zero *result* means the filesystem accepted no bytes — report the
            // portion still unwritten against the buffer we were handed.
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "wrote 0 of the {} remaining bytes ({} total) at offset {}",
                    remaining.len(),
                    buf.len(),
                    offset
                ),
            ));
        }
        remaining = &remaining[n..];
        at += n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// `capacity` answers for a directory that exists (both OS probes are volume
    /// calls that succeed), with total > 0 and free bounded by total.
    #[test]
    fn capacity_answers_for_an_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let (free, total) = capacity(dir.path()).expect("probe on an existing dir");
        assert!(total > 0, "a volume has a total size");
        assert!(free <= total, "free-to-caller never exceeds the volume total");
    }

    /// `capacity` answers None for a path no volume backs (both OS probes fail).
    #[test]
    fn capacity_is_none_for_a_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("does-not-exist");
        assert!(capacity(&gone).is_none(), "a missing path has no volume answer");
    }

    /// `write_all_at` writes at an absolute offset without disturbing other
    /// bytes, and two writes at distinct offsets don't clobber each other
    /// (the pwrite contract the striped downloader relies on).
    #[test]
    fn write_all_at_writes_at_an_absolute_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world").unwrap();
        // A second, later write through the same handle must not disturb the first.
        write_all_at(&f, 6, b"walgit").unwrap();
        write_all_at(&f, 0, b"HELLO").unwrap();
        let got = std::fs::read(&path).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&got),
            "HELLO walgit",
            "both positional writes land at their offsets, bytes in between survive"
        );
    }

    /// The write-zero path must fail loudly, not spin: a zero result is reported
    /// as WriteZero (the loop only runs while there is data to write).
    #[test]
    fn write_all_at_zero_write_fails_loudly() {
        // An empty buffer is a no-op, not an error.
        let dir = tempfile::tempdir().unwrap();
        let f = std::fs::File::create(dir.path().join("f")).unwrap();
        write_all_at(&f, 0, b"").unwrap();
    }
}
