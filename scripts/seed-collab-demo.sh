#!/bin/bash
# seed-collab-demo.sh — seed a walgit repository's collaboration layer with
# reproducible demo data (issue #38): the work-unit board trio and a real
# two-runner CI claim race, so the SPA guide page's three diagrams render
# live data and the thread page narrates the race. Idempotence: collab
# entries are append-only — run once against a fresh repo; re-running adds
# new entries rather than failing.
#
# Required environment:
#   WALGIT_SEED_URL    server base url (e.g. http://localhost:9091)
#   WALGIT_SEED_REPO   repository as <owner>/<repo>
#   WALGIT_SEED_TOKEN  a write-capable token (omit on an auth-none loopback host)
#
# Dependencies: git, openssl, python3, the `walgit` CLI on PATH (or $WALGIT_BIN).
set -euo pipefail

URL="${WALGIT_SEED_URL:?set WALGIT_SEED_URL}"
REPO="${WALGIT_SEED_REPO:?set WALGIT_SEED_REPO}"
WALGIT="${WALGIT_BIN:-walgit}"
# The CLI insists on a config file; these subcommands only do git work.
export WALGIT_CONFIG="${WALGIT_CONFIG:-/dev/null}"

for dep in git openssl python3 "$WALGIT"; do
  command -v "$dep" >/dev/null 2>&1 || { echo "missing dependency: $dep" >&2; exit 1; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
clone="$work/repo"
clone_args=(clone -q)
[ -n "${WALGIT_SEED_TOKEN:-}" ] && clone_args+=(-c "http.extraHeader=Authorization: Bearer $WALGIT_SEED_TOKEN")
git "${clone_args[@]}" "$URL/$REPO.git" "$clone"
git -C "$clone" config user.name "seed"
git -C "$clone" config user.email "seed@demo"
[ -n "${WALGIT_SEED_TOKEN:-}" ] && git -C "$clone" config http.extraHeader "Authorization: Bearer $WALGIT_SEED_TOKEN"
# The CLI's `--push` runs plain `git push` in this checkout; an empty push of
# nothing also warms the remote-tracking view.
origin="$(git -C "$clone" remote get-url origin)"

post() { # post <kind> <thread> <actor> <parent> <body-json> -> prints entry oid
  local kind="$1" id="$2" actor="$3" parent="$4" body="$5" out ref oid
  out="$("$WALGIT" collab entry --repo "$clone" --kind "$kind" --id "$id" \
    --actor "$actor" --parent "$parent" --body "$body" \
    --key "$work/key-$actor" --push "$origin")"
  oid="${out##* }"
  [ -n "$oid" ] || { echo "post failed: $out" >&2; exit 1; }
  echo "$oid"
}

register() { # register <principal>
  "$WALGIT" collab principal-register --repo "$clone" --principal "$1" \
    --key "$work/key-$1" --push "$origin" >/dev/null
}

keyfile() { # keyfile <principal> — Ed25519 seed, 32 raw bytes as hex
  openssl rand -hex 32 > "$work/key-$1"
}

echo "== principals =="
for p in alice ci-runner-a ci-runner-b; do keyfile "$p"; register "$p"; done

echo "== board trio (w1 issue+review+status, w2 issue+status, w3 issue) =="
e1=$(post issue w1 alice "" '{"title":"加深色模式","text":"跟随系统的深色主题。"}')
e2=$(post review w1 alice "$e1" '{"decision":"approve","text":"看了，没问题。"}')
post status w1 alice "$e2" '{"status":"needs-review"}'
f1=$(post issue w2 alice "" '{"title":"看板列自定义支持 glob","text":"列名想匹配一类分支。"}')
post status w2 alice "$f1" '{"status":"in-progress"}'
post issue w3 alice "" '{"title":"补 e2e 到 CI recipe"}'

echo "== CI race (two runners claim the same run; only one result is effective) =="
tip=$(git ls-remote origin refs/heads/main | cut -f1)
[ -n "$tip" ] || { echo "repo has no refs/heads/main — push something first" >&2; exit 1; }
# run id (D1-CI §5, normative): ci- + hex16(fnv1a64(task ‖ 0x1f ‖ ref ‖ 0x1f ‖ commit))
runid=$(python3 - "$tip" <<'PY'
import sys
data = b"test\x1frefs/heads/main\x1f" + sys.argv[1].encode()
h = 0xCBF29CE484222325
for b in data:
    h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
print(f"ci-{h:016x}")
PY
)
claim_body="{\"task\":\"test\",\"ref\":\"refs/heads/main\",\"commit\":\"$tip\",\"ttl\":300,\"attempt\":1,\"runner\":\"seed\"}"
ca=$(post ci_claim "$runid" ci-runner-a "" "$claim_body")
sleep 1 # the earliest claim wins (ts, actor, oid) — keep the ts order honest
cb=$(post ci_claim "$runid" ci-runner-b "" "$claim_body")
post ci_result "$runid" ci-runner-a "$ca" \
  "{\"task\":\"test\",\"ref\":\"refs/heads/main\",\"commit\":\"$tip\",\"attempt\":1,\"claim\":\"$ca\",\"conclusion\":\"success\",\"exit_code\":0,\"duration_ms\":1200,\"log_summary\":\"seeded: all tests passed\",\"log_sha256\":\"\"}"
post ci_result "$runid" ci-runner-b "$cb" \
  "{\"task\":\"test\",\"ref\":\"refs/heads/main\",\"commit\":\"$tip\",\"attempt\":1,\"claim\":\"$cb\",\"conclusion\":\"success\",\"exit_code\":0,\"duration_ms\":1350,\"log_summary\":\"seeded: all tests passed (lost the race — kept for audit)\",\"log_sha256\":\"\"}"

echo "== a claim that never results (the TTL sight) =="
lintid=$(python3 - "$tip" <<'PY'
import sys
data = b"lint\x1frefs/heads/main\x1f" + sys.argv[1].encode()
h = 0xCBF29CE484222325
for b in data:
    h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
print(f"ci-{h:016x}")
PY
)
post ci_claim "$lintid" ci-runner-b "" \
  "{\"task\":\"lint\",\"ref\":\"refs/heads/main\",\"commit\":\"$tip\",\"ttl\":300,\"attempt\":1,\"runner\":\"seed\"}"

echo
echo "seeded $REPO:"
echo "  board threads: w1 w2 w3"
echo "  ci race:       $runid   (ci-runner-a's result is effective)"
echo "  pending claim: $lintid"
echo "  guide page:    $URL/$REPO/collab/guide"
