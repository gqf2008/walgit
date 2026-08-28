# Repository instructions for AI coding agents

walgit is a git server (one binary in front of an object store) with a WAL-backed
storage engine, developed as an **agent-native collaboration platform**. Read the
architecture and operating manual before touching anything: `AGENTS.md` (constraints
§1, WAL design §2, principles §3, decisions §4, working rules §5, **agent protocol
§6**), then `GOAL.md` for what it is for, `CONTRIBUTING.md` for the workflow. These
outrank any generic convention.

## Working here

- **Work units are issues.** Batches (`batch` label) own a checklist; bugs and tasks
  use the forms. Claim an issue (label `in-progress` + comment naming your worktree)
  before starting; one worktree per issue.
- **Never push to `main` directly** — the ruleset rejects it. Work in a worktree,
  open a PR with the template (Verification / Model Used / reviewer), and let
  auto-merge finish after CI.
- **Commits are Conventional Commits** (`fix(git): …`, one logical change each,
  message says why).
- **`just ci`** = warnings + clippy + test + e2e + sim — everything a merge needs.
  Run the relevant tiers locally before pushing (Windows notes: `docs/WINDOWS.md`).

## Reading CI

The CI workflow comments a summary on every PR. `clippy (known-red debt)` is
expected-red (~1300 pre-existing hits, issue #1; keep your increment at zero).
Tests on the known-flaky list (§5) auto-rerun once. Anything else red is a
regression — investigate.

## Safety

- `fail closed` is a hard contract (§1.3): auth, policy, integrity checks never
  silently degrade.
- Secrets and credentials never go in issues, PRs, or comments — see SECURITY.md
  for private reporting.
- Don't weaken lints, thresholds, or tests to make CI green.
