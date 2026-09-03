# SKILL.md — working with this walgit host as an AI agent

walgit is a git smart-HTTP server whose repositories live in an object store:
hosts are disposable caches, the bucket is the repository. This file teaches
an AI agent to discover, read, and write repositories on **this host** with
plain HTTP and plain git — no interactive steps anywhere.

## Discover

- `GET /api/v1` — the discovery document (lanes, endpoint list).
- `GET /api/v1/owners` → `["owner", …]`
- `GET /api/v1/owners/{owner}/repos` → `["repo", …]`
- `GET /repos.js` — the browser SDK (`window.repos`; `/repos.mjs` for ESM).
- Web UI: `/repos` (host-wide repository index), `/{owner}/{repo}` (browser).

## Git: clone / fetch / push

- Remote URL: `http(s)://<this-host>/<owner>/<repo>.git`.
- Auth modes: `none` (loopback only, everyone is anon), `token`, `oidc`. On a
  token/oidc host every request carries `Authorization: Bearer <token>`; the
  one-command client setup is
  `sh -c "$(curl -fsSL <host>/services/setup.json | …)"` — see
  `/services/setup.json` for the exact recipes, or run the installer:
  `sh -c "$(curl -fsSLk '<host>/services/public/install.sh')" -- <owner>/<repo>`.
- Big repositories advertise **bundle-uri**: clone with
  `-c transfer.bundleURI=true` (bytes come from static bundles, not
  upload-pack). Bounded fetches (CI checkouts with `--depth`/`--filter`) may
  pass `-c transfer.bundleURI=false`. Blobless: `--filter=blob:none`.

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
- A per-repo **push policy** may protect refs (rules at
  `repos/<owner>/<repo>/policy.json`); an empty/missing policy means anyone
  with write may move any ref.

## LFS

- Batch + basic transfer under `/{owner}/{repo}.git/info/lfs`; objects are
  sha256-addressed, immutable.

## Rules of thumb

- `404` is a cheap probe answer — probe keys, don't list.
- Immutable objects answer with `Cache-Control: immutable`, a strong `ETag`,
  and `Range` support; ref-dependent answers carry SWR + `ETag`.
- A push acknowledged by git is already visible to the next read on any host.
