# Issue M0-001 — Create repository skeleton and directory layout

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** —

## Context

The repository currently holds only `docs/`. Every later issue assumes a
predictable layout, so it is created once, deliberately, rather than accreting.

## Goal

Create the top-level directory structure and supporting files that the rest of M0 fills in.

## Scope

- `crates/`, `deploy/`, `migrations/edge/`, `migrations/cloud/`, `scripts/`, `test/fixtures/`, `test/scenarios/`, `tools/`
- `.gitignore` covering Rust, SQLite, secrets, Tauri, and ESP build output
- `.env.example` with placeholder values for every secret the system will use
- `.gitattributes` — **already exists** from the planning phase; verify it still covers every file type M0 introduces (`.sh`, `.conf`, `.sql`, `.yml`, Dockerfile)

## Non-goals

- Any Rust code (M0-002).
- Any Docker file (M0-009).

## Dependencies

- Nothing — this issue can start immediately.

## Implementation notes

`.gitignore` must exclude `deploy/mosquitto/passwd` and `.env` — both will
contain real secrets from M0-008 onward. `.env.example` documents their shape
with obvious placeholders so nobody has to guess a variable name.

Line-ending normalisation matters here: shell scripts and the Mosquitto config
are consumed inside Linux containers, and CRLF breaks both in confusing ways.

`.gitattributes` was created during planning and already pins `* text=auto
eol=lf`, with `.ps1`/`.bat`/`.cmd` kept as CRLF and binary types marked. It is
repo-local and explicit, so behaviour does not depend on a developer's
`core.autocrlf`. This issue only extends it if M0 introduces a file type it does
not yet cover — do not weaken the `eol=lf` default.

## Acceptance criteria

- [x] The directory tree exists and is committed (empty dirs carry a `.gitkeep`).
- [x] `.gitignore` excludes `target/`, `*.sqlite*`, `.env`, `deploy/mosquitto/passwd`, `ui/**/dist/`, `firmware/**/.embuild/`.
- [x] `git ls-files --eol` reports `i/lf w/lf` for every tracked file.
- [x] `git add .` produces no CRLF/LF warnings.
- [x] `.env.example` lists every variable with a placeholder and no real value.
- [x] `git status` is clean after a build produces artefacts.

## Verification

```bash
git status --porcelain   # empty after a build
git check-ignore -v .env deploy/mosquitto/passwd
git ls-files --eol | awk '{print $1,$2}' | sort -u   # expect only: i/lf w/lf
```

## Tests required

- None — structural change. Verified by `git check-ignore` and `git ls-files --eol`.

## Documentation impact

- None; the layout is already described in docs/architecture/component-model.md.

## Files likely affected

```text
.gitignore
.gitattributes
.env.example
crates/.gitkeep
deploy/.gitkeep
migrations/edge/.gitkeep
migrations/cloud/.gitkeep
scripts/.gitkeep
test/fixtures/.gitkeep
test/scenarios/.gitkeep
```
