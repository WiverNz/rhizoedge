# Issue M12-004 — Implement the plant detail view

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-002

## Context

Where the operator decides whether to trust the system: current values, the
recommendation, and — crucially — **why**.

## Goal

Show one plant completely and explainably.

## Scope

- Current values with **age**, greyed when stale
- Plant and irrigation state
- Recommendation with its reasons rendered in plain language
- Water budget: delivered in 24 h against the cap
- Last watering
- Lockout with reason and what clears it
- **Stale data shown as stale, never as a fresh-looking number**

## Non-goals

- Charts (M12-007).
- Actions (M12-006).

## Dependencies

- M12-002

## Implementation notes

Showing a stale reading as if current is the presentation failure with real
consequences: it produces exactly the wrong human response, since the operator
concludes the plant is fine when the system has actually lost sight of it.

Reasons are rendered from the API's structured values — the UI does not
recompute them.

## Acceptance criteria

Apply the prototype subset specified in the milestone PRD. Leave any broader
criterion unchecked and explicitly deferred; completion of this slice does not
claim the deferred full scope passed.

- [ ] All current values render with their age.
- [ ] **Stale values are visibly greyed with the age shown.**
- [ ] The recommendation renders with its reasons in plain language.
- [ ] The water budget shows delivered against the cap.
- [ ] A lockout shows its reason and what clears it.
- [ ] Reasons come from the API, not from UI logic.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri dev   # manual inspection
```

## Tests required

- Component tests: stale rendering, reason rendering, lockout rendering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/plant.rs
```
