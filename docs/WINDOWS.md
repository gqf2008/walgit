# Windows — build, test, dev-store on a Windows host

Context: **runbook** for developing walgit on Windows. The fork keeps the
platform first-class: CI runs a windows leg (compile all targets + fast tier
+ sim + e2e, `.github/workflows/ci.yml`), and the local workflow below is the
same surface a contributor gets.

## 1. Prerequisites

- **Rust**: `rustup` (the stable toolchain installs on
  first `cargo` use; `rustup show` in the repo root).
- **protoc**: `choco install protoc -y` (prost-build does not vendor it).
- **git for Windows**: required — the server shells out to real `git`
  (`multi-pack-index write`, `repack`, `index-pack`, …). Any recent build works.
- **pnpm**: `corepack enable` or a standalone install; `just web-build` uses it.
- **just**: optional for a plain build; `just` itself is not installed by any
  package manager on Windows — grab a release binary from
  `just.systems` or `cargo install just`. (CI installs it via
  `taiki-e/install-action`.) Git Bash is the recommended shell for `just`
  recipes: they use GNU coreutils (`timeout`, `setsid`, …) that Git Bash ships.

Everything below assumes **Git Bash** (comes with git for Windows) unless
noted. `cargo` and `rustc` are the rustup proxies on PATH.

## 2. Build and test

```bash
just web-build        # SPA + SDK (pnpm install --frozen-lockfile + vite build)
just test             # fast hermetic tier (< 1 min)
just e2e              # smart-HTTP end-to-end against real git
just sim              # fault-injection simulation suite (seeds: WALGIT_SIM_SEEDS)
just ci               # warnings + clippy + test + e2e + sim
```

Notes specific to Windows:

- `just` recipes wrap long commands in `timeout` (GNU coreutils) — Git Bash
  provides it; on a shell without it the recipes degrade to running without a
  watchdog.
- The e2e suite builds a `git` shim with `rustc` on the fly for the
  history-pack stall test (CreateProcess resolves only `.exe`, so a script
  cannot shadow `git`); a missing rustc makes that one test print the reason
  and skip.
- git for Windows marks finished `pack-*.pack`/`pack-*.idx` **READ_ONLY**.
  Supersede deletes and repo teardown clear the attribute before removing
  (see `LocalRepo::remove_pack_file` and `Registry::delete`); if you ever hit
  `os error 5` deleting pack files outside those paths, clear the attribute
  first.

## 3. `just dev-store` on Windows (rustfs)

`just dev-store` is a nix-podman recipe (rootless, unix sockets) and does not
run on Windows. Two equivalents:

**Option A — podman-desktop / docker**: install either, then

```bash
podman compose up -d rustfs     # or: docker compose up -d rustfs
podman compose run --rm create-bucket
```

The compose file starts rustfs (S3-compatible) on `127.0.0.1:9000` with
credentials `walgit-dev / walgit-dev-secret` and bucket `walgit-test`.

**Option B — rustfs binary directly**: run a rustfs binary (any S3-compatible
target — memory, minio, local FS) on `:9000` and create the bucket:

```bash
rustfs --listen 127.0.0.1:9000 --backend memory &
# create the bucket with any S3 client (mc, aws cli, …):
#   walgit-test, walgit-dev / walgit-dev-secret
```

Then start the server with the standalone config:

```bash
cargo build --release --bin walgit-server
./target/release/walgit-server --config walgit.standalone.toml
```

(`walgit.standalone.toml` points the store at `http://127.0.0.1:9000` with
those credentials.)

## 4. NTFS symlinks (store-mount tests)

The `test_serve_level_links_base_from_store_mount` case (`crates/walgit-wal`
tests) links a base pack from a read-only bucket mount via a real NTFS
symbolic link. The test detects whether symlinks can be created and prints
`skipped: creating symlinks failed (Windows needs Developer Mode or
administrator rights)` when they cannot.

On a current Windows 10/11 (verified: 10.0.19045, non-admin, Developer Mode
off) ordinary users can create symlinks on NTFS without any setting, so the
case runs for real and is covered by the fork's windows leg. If you ever see
the skip message, enable Developer Mode:

> Settings → Privacy & security → For developers → Developer Mode: On

(the registry key is
`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock\AllowDevelopmentWithoutDevLicense = 1`).

The repo itself may live on any filesystem (tests use `%TEMP%`, NTFS by
default); a symlink-capable volume is only needed for the mount-link case.

## 5. What CI covers (fork, issue #2)

- **ubuntu** build-test: `just ci` (warnings, clippy, test, e2e, sim).
- **windows** leg: compiles every target, runs the fast tier, sim and e2e
  (`cargo`, not `just` — the job exists so platform seams drift loudly).
- Known flaky on both platforms (rerun, not skip): `sim::base_rebuild…`
  ~1 in 7 (shared `TEST_ABORT_AFTER`), `fetch_from_front_…` ~1 in 3 under the
  full e2e suite; both pass alone.
