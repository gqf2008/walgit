//! Test-only plumbing shared by the CLI's `#[cfg(test)]` modules: the two-stage
//! git pipeline the suites used to spell through `/bin/sh`, which does not exist
//! on Windows (and every integration binary of a workspace crate is its own
//! world, so this small helper is duplicated rather than exported as API).

#![cfg(test)]

/// `git <first> | git <second>` without a POSIX shell. Collects the first
/// process's whole output, then feeds it to the second through its stdin:
/// suites here exchange small packs, and a buffered hop avoids both pipe-volume
/// deadlocks and the platform quirks of piping two live children directly
/// (observed flaky on Windows: an instantly-empty read from a healthy upstream).
/// A failure names its cause instead of yielding a silent empty pack.
pub(crate) fn git_pipe(
    cwd: &std::path::Path,
    first: &[&str],
    second: &[&str],
) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let up = Command::new("git")
        .args(first)
        .current_dir(cwd)
        .stderr(Stdio::piped())
        .output()
        .expect("run upstream git");
    if !up.status.success() {
        panic!(
            "git {first:?} failed: {} — stderr: {}",
            up.status,
            String::from_utf8_lossy(&up.stderr)
        );
    }

    let mut down = Command::new("git")
        .args(second)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn downstream git");
    // The whole input is already in memory: write it, close stdin, then wait.
    // A write error means the child died early — its status says how.
    if let Some(mut stdin) = down.stdin.take() {
        let _ = stdin.write_all(&up.stdout);
    }
    let out = down.wait_with_output().expect("wait downstream git");
    if !out.status.success() {
        panic!(
            "git {second:?} failed: {} — stderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out
}
