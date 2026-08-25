# Issue M12-010 — Implement connection state handling

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-003

## Context

PRD 120: an operator checking on a plant during a network problem needs to see
the last known state **with its age** — never a blank screen, and never a stale
value presented as current.

## Goal

Degrade legibly when the edge is unreachable.

## Scope

- A banner naming the unreachable URL
- **Last known data shown greyed with its age**
- Reconnection with backoff, and a visible attempt count
- 503 rendered as 'controller starting' with the failing readiness checks
- Manual retry

## Non-goals

- Offline caching beyond the last response.

## Dependencies

- M12-003

## Implementation notes

Never a blank screen. A blank screen tells the operator nothing and invites
them to assume the worst or the best arbitrarily; greyed data with an age tells
them exactly what is known and how old it is.

Rendering a 503 with the failing readiness checks turns 'it is broken' into 'the
broker is unreachable', which is actionable.

## Acceptance criteria

- [ ] An unreachable edge shows a banner naming the URL.
- [ ] **Last known data is shown greyed with its age.**
- [ ] Reconnection is attempted with backoff and is visible.
- [ ] A 503 renders the failing readiness checks.
- [ ] Manual retry works.
- [ ] The screen is never blank.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri dev   # then stop the edge
```

## Tests required

- Component tests: banner, greyed data, 503 rendering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/state/connection.rs
```
