//! Make `cargo build`/`cargo test` work in a fresh checkout without running the
//! web build. `rust-embed` requires `../../web/dist` to exist at compile time;
//! when the SPA has not been built (`just web-build`) we drop in a placeholder
//! `index.html` so the server compiles and the HTML routes still answer 200.
//! A real `pnpm run build` overwrites the placeholder.

use std::fs;
use std::path::Path;

const PLACEHOLDER: &str = "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>walgit</title></head>\n\
<body><p>walgit web UI is not built in this binary. Run <code>just web-build</code> (vite via pnpm) and rebuild.</p></body></html>\n";

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    watch_git_head(manifest);
    println!("cargo:rustc-env=WALGIT_BUILD_SHA={}", build_sha());
    let dist = manifest.join("../../web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let index = dist.join("index.html");
    if !index.exists() {
        fs::create_dir_all(&dist).expect("create web/dist");
        fs::write(&index, PLACEHOLDER).expect("write placeholder web/dist/index.html");
        println!(
            "cargo:warning=web/dist was missing; wrote a placeholder index.html (run `just web-build` for the real UI)"
        );
    }
}

/// Rerun this script when the checked-out commit moves. `git pull`, a branch
/// switch or a rebase change `.git/HEAD` or the ref it points to; without
/// watching them, an incremental build after a pull keeps the stale
/// `WALGIT_BUILD_SHA` and `/healthz` + `walgit --version` lie about which
/// commit is running (#37). Worktrees: `.git` is a file, so resolve
/// `--git-dir` (per-worktree HEAD) and `--git-common-dir` (shared refs).
/// Missing paths are fine: cargo reruns when they appear. An archived source
/// tree or a container build has no `.git` — nothing to watch.
fn watch_git_head(manifest: &Path) {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(manifest)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let Some(git_dir) = git(&["rev-parse", "--git-dir"]) else {
        return;
    };
    let git_dir = manifest.join(git_dir);
    let head = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    // HEAD reads `ref: <name>`: the ref file lives in the COMMON dir
    // (worktrees share refs); a detached HEAD only moves the HEAD file.
    if let Ok(content) = fs::read_to_string(&head)
        && let Some(name) = content.trim().strip_prefix("ref: ")
    {
        let common = git(&["rev-parse", "--git-common-dir"])
            .map_or_else(|| git_dir.clone(), |c| manifest.join(c));
        println!("cargo:rerun-if-changed={}", common.join(name).display());
        // A fully packed checkout has no loose ref file to move; watch
        // packed-refs too so even that layout reruns on a pull.
        println!(
            "cargo:rerun-if-changed={}",
            common.join("packed-refs").display()
        );
    }
}

/// Build identity for `/healthz` (`version`) and `walgit --version`: the commit
/// the binary was built from. A container or package build may pass it as
/// `WALGIT_BUILD_SHA` (an archived source tree has no `.git`); a checkout
/// falls back to `git rev-parse --short=12 HEAD`; otherwise "dev".
fn build_sha() -> String {
    println!("cargo:rerun-if-env-changed=WALGIT_BUILD_SHA");
    if let Ok(s) = std::env::var("WALGIT_BUILD_SHA") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dev".to_string())
}
