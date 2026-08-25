# Issue M0-011 — Adopt rhizo-docscheck into the workspace

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002

## Context

The planning artefacts cross-reference heavily: PRDs cite ADRs, issues cite
PRDs, and everything cites SAFETY-nnn. Broken references accumulate silently and
make the documentation untrustworthy exactly when someone needs it.

**The validator already exists.** It was written during the planning phase and
lives at `tools/docscheck/` as a standalone, dependency-free Rust crate. It
currently runs via:

```bash
cargo run --manifest-path tools/docscheck/Cargo.toml
```

This issue does not write it. It **adopts** it: makes it a member of the
workspace created in M0-002, so the gate command becomes uniform with the rest
of the project.

## Goal

Bring the existing planning validator into the Rust workspace and keep it green.

## Scope

- Add `tools/docscheck` to the root workspace `members`
- Ensure it builds under the pinned 1.98.0 toolchain and passes `clippy -D warnings`
- Adopt `lints.workspace = true` like every other member
- Confirm it still runs clean against the current documentation
- Keep it dependency-free so it works offline

## Non-goals

- Rewriting or re-designing the validator — it exists and works.
- Prose quality or completeness checks.
- Rewriting documentation automatically.
- Making it a product crate. It validates planning artefacts and is never
  shipped or depended on by any runtime crate.

## Dependencies

- M0-002

## Implementation notes

The tool already validates:

- required ADR files exist; ADR ids are unique
- required PRDs exist (000–140, one per milestone); PRD ids are unique
- `docs/issues/M0`…`M14` exist; issue ids are unique within a milestone
- every referenced `M<n>-NNN`, `ADR-NNN`, `PRD NNN`, `SAFETY-NNN`, `SCEN-NNN`
  resolves to an artefact that exists
- every SAFETY invariant has a section and a "Planned tests" subsection
- required architecture, protocol, and testing files exist
- ROADMAP.md lists every milestone and states the real issue counts
- relative markdown links resolve
- the issue dependency graph is acyclic
- issue numbering is a valid execution order (dependencies point to lower
  numbers in the same milestone, or to earlier milestones)

Source-input documents (the two `*_Prompt.md` files) are deliberately excluded
from reference validation: their illustrative examples are not real references.

Once it is a workspace member, `cargo run -p rhizo-docscheck` works and the gate
command in ROADMAP.md §5 can be simplified accordingly — update it in the same
change so the documented command matches reality.

Keep the crate dependency-free. It must run in CI with no network and no
registry access, before any other crate compiles.

## Acceptance criteria

- [ ] `tools/docscheck` is a member of the root workspace.
- [ ] `cargo run -p rhizo-docscheck` exits 0 against the current documentation.
- [ ] `cargo clippy -p rhizo-docscheck -- -D warnings` is clean.
- [ ] It has no third-party dependencies.
- [ ] It exits non-zero with a specific message for: a broken relative link, a
      safety-invariant reference with no registry entry, a duplicate ADR id, a
      duplicate issue id, and a dependency on a non-existent issue.
- [ ] All failures are reported in a single run, not one per invocation.
- [ ] ROADMAP.md §5 and docs/README.md state the `-p rhizo-docscheck` form.

## Verification

```bash
cargo run -p rhizo-docscheck
echo $?
cargo clippy -p rhizo-docscheck --all-targets -- -D warnings
```

## Tests required

- Manual: introduce each failure class listed above, confirm the tool detects it
  with a specific message, then revert.

## Documentation impact

- ROADMAP.md §5 gate command.
- docs/README.md validation section.

## Files likely affected

```text
Cargo.toml
tools/docscheck/Cargo.toml
tools/docscheck/src/main.rs
ROADMAP.md
docs/README.md
```
