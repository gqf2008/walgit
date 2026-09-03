# SKILL.md — working with this walgit host as an AI agent

walgit is a git smart-HTTP server whose repositories live in an object store:
hosts are disposable caches, the bucket is the repository. Collaboration
(issues, PRs, boards) and CI are **inside the repository** — signed entries on
`refs/collab/*` — and the `walgit` CLI is their primary interface. This file
teaches an AI agent to discover, read, and write repositories on **this host**
with plain git, the CLI, and HTTP. No interactive steps anywhere.

## Discover

- `GET /api/v1` — the discovery document (lanes, endpoint list).
- `GET /api/v1/owners` → `["owner", …]`; `GET /api/v1/owners/{owner}/repos` →
  `["repo", …]` (or `walgit repo list`).
- `GET /repos.js` — the browser SDK (`window.repos`; `/repos.mjs` for ESM).
- Web UI: `/repos` (host-wide repository index), `/{owner}/{repo}` (browser).

## Git: clone / fetch / push

- Remote URL: `http(s)://<this-host>/<owner>/<repo>.git`.
- Auth modes: `none` (loopback only, everyone is anon), `token`, `oidc`. On a
  token/oidc host every request carries `Authorization: Bearer <token>`; the
  one-command client setup is in `/services/setup.json`, or run the installer:
  `sh -c "$(curl -fsSLk '<host>/services/public/install.sh')" -- <owner>/<repo>`.
- Big repositories advertise **bundle-uri**: clone with
  `-c transfer.bundleURI=true` (bytes come from static bundles, not
  upload-pack). Bounded fetches (CI checkouts with `--depth`/`--filter`) may
  pass `-c transfer.bundleURI=false`. Blobless: `--filter=blob:none`.

## The CLI: collaboration, CI, management (`walgit …`)

One binary, subcommands by role. Everything below works against a **checkout**
of a repository on this host (clone it first); entries are Ed25519-signed and
pushed as ordinary refs, so a local write becomes visible with one `--push`.

### D1 collaboration — issues, PRs, the board (`walgit collab …`)

- Read (JSON out — pipe through `jq`):
  - `walgit collab ls` — thread ids on `refs/collab/inbox/*`.
  - `walgit collab thread <id>` — one thread, parent-ordered, per-entry
    signature verification.
  - `walgit collab pr <id>` — aggregated PR view + merge-rule evaluation.
  - `walgit collab board` — the work-unit board: threads projected under
    `.walgit/board.toml`. Read-only — moving a card is a signed `status`
    entry, not an edit.
  - `walgit collab report` — global dashboard: threads, PR status,
    verification health, activity.
- Write (construct + sign + deliver):
  - First use only: `walgit collab principal-register` — publish your
    principal's public key.
  - `walgit collab entry --kind <kind> --id <thread> --actor <principal>
    --body '<json>' --key <ed25519-hex> [--base … --head …] --push <remote>`
    — appends a signed entry and pushes it. Nobody edits state; a change is
    a new signed entry whose parent chain anyone can replay and verify.
- Automate: `walgit collab watch --exec <cmd>` — resident loop: fetch
  `refs/collab/*`, invoke `cmd` with each new/changed entry's JSON on stdin.

### Decentralized CI (`walgit ci …`)

The server holds no CI logic. A runner is a client:

- `.walgit/ci.toml` in the tested commit declares the tasks;
  `walgit ci validate` checks it.
- `walgit ci run` — subscribe to ref tips, claim runs with signed `ci_claim`
  entries, execute, publish signed results. Simultaneous claims are a legal
  race: both may run, exactly one result is effective (deterministic winner
  rule), the others are kept for audit.
- `walgit ci status` — every run in the checkout's collab log, aggregated.

### Repository management & ops

- `walgit repo create|list|info` — manage repositories.
- `walgit repo policy` — per-repo push rules (`policy.json`).
- `walgit repo settings` — per-repo TOML overrides (`[bundles]`,
  `[maintenance]`, `[compaction]`) published through the WAL.
- `walgit wal ls|show|materialize` — provenance: every push, repack,
  checkpoint is a log entry you can read and replay.
- `walgit mirror` — follow another git host's refs into walgit.
- `walgit import` — import an existing repository; `walgit compact`,
  `walgit bundle` — maintainer operations (compaction, static bundles).

## HTTP API

- Repository-scoped: `/{owner}/{repo}/api/…` — `refs`, `resolve`, `tree`,
  `blob`, `commits`, `commit`, `overview`, `tasks`, `settings`, `collab/*`.
  Credential: a bearer token or the same-origin session cookie.
- Cross-origin browser lane: `/{owner}/{repo}/api-browser/…`
  (`credentials: "include"`; CORS only for the host's configured origins).
- Any JSON endpoint that cannot answer immediately streams the **SSE
  envelope** when the request sends `Accept: text/event-stream` — read the
  stream (`progress` / `notice` / terminal `result`|`error`), never poll
  blindly.
- Long work is a **task**: `GET /{owner}/{repo}/api/tasks`, live packet
  stream at `…/tasks/{id}`.
- `503` + `Retry-After: n` means this host does not serve that repository's
  object work right now — retry after the advertised seconds; refs-level
  reads stay available on every host.

## Writing (push)

- Push with git over the same remote URL. Refs are linearized by a manifest
  compare-and-swap; a moved ref is rejected per-ref with its actual old value.
- A per-repo **push policy** may protect refs; an empty/missing policy means
  anyone with write may move any ref.

## LFS

- Batch + basic transfer under `/{owner}/{repo}.git/info/lfs`; objects are
  sha256-addressed, immutable.

## Rules of thumb

- `404` is a cheap probe answer — probe keys, don't list.
- Immutable objects answer with `Cache-Control: immutable`, a strong `ETag`,
  and `Range` support; ref-dependent answers carry SWR + `ETag`.
- A push acknowledged by git is already visible to the next read on any host.
- When in doubt, read the rules, not the platform: every collaboration screen
  is the same signed entries computed through public rules — the CLI prints
  the same bytes any server computes.
