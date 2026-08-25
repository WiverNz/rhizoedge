# Issue M0-004 — Establish error type and failure classification conventions

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002

## Context

ADR-014 requires every failure to be classified as Transient, Permanent, or
Fatal, and requires the classification to be a function rather than scattered
match arms. The convention is established now so no crate invents its own.

## Goal

Define the shared error conventions and the `FailureKind` classification type.

## Scope

- `FailureKind` enum in `rhizo-telemetry`
- A documented convention: `thiserror` in libraries, `anyhow` only at binary top level
- A `Classify` trait that error types implement
- Doc comments stating the exhaustive-match rule

## Non-goals

- Concrete error types for storage, MQTT, or cloud — each lands with its crate.
- Retry logic (M0-007).

## Dependencies

- M0-002

## Implementation notes

```rust
pub enum FailureKind { Transient, Permanent, Fatal }
pub trait Classify { fn classify(&self) -> FailureKind; }
```

The rule that keeps this honest: every `Classify` impl matches exhaustively with
**no catch-all arm**, so a new error variant fails to compile until someone
decides whether it is retryable. Document that rule next to the trait, because
it is the entire point.

## Acceptance criteria

- [ ] `FailureKind` and `Classify` exist and are documented.
- [ ] The doc comment states the no-catch-all rule explicitly.
- [ ] The convention (thiserror vs anyhow) is documented in the crate docs.

## Verification

```bash
cargo doc -p rhizo-telemetry --no-deps
cargo test -p rhizo-telemetry
```

## Tests required

- A sample error type implementing `Classify`, with one test per variant.

## Documentation impact

- Crate-level docs on `rhizo-telemetry` stating the conventions.

## Files likely affected

```text
crates/telemetry/src/failure.rs
crates/telemetry/src/lib.rs
```
