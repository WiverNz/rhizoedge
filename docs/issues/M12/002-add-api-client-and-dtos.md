# Issue M12-002 — Add the API client and shared DTOs

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-001

## Context

The UI consumes the Edge REST API exclusively. Sharing DTO types with the
edge avoids the classic drift where the UI parses a field the API renamed.

## Goal

Provide a typed client for the edge API.

## Scope

- A shared DTO crate used by both the edge API layer and the UI
- An async client wrapping `fetch`
- The error envelope deserialised into a typed error
- **409 mapped to a distinct `Refused` variant, not a generic error**
- Configurable base URL

## Non-goals

- Any business logic.

## Dependencies

- M12-001

## Implementation notes

`Refused` as a distinct variant from `Error` is the type-level expression of
PRD 120's state model: a safety refusal is the system working correctly, and
presenting it as a malfunction teaches the operator to distrust correct
behaviour.

Sharing DTOs means an API field rename breaks the UI build rather than producing
a silently empty field.

## Acceptance criteria

- [ ] All endpoints have typed client methods.
- [ ] The error envelope deserialises into a typed error.
- [ ] **409 produces `Refused` carrying the lockout reason.**
- [ ] Other failures produce `Error`.
- [ ] The base URL is configurable.
- [ ] DTOs are shared with the edge, so a rename breaks the build.

## Verification

```bash
cd ui/rhizo-ui && cargo test client::
```

## Tests required

- Deserialisation per endpoint.
- **409 to Refused mapping.**
- Error handling.

## Documentation impact

- None.

## Files likely affected

```text
crates/api-dto/src/lib.rs
ui/rhizo-ui/src/client.rs
```
