# Issue M12-006 — Implement watering actions and refusal handling

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-004

## Context

**The safety-critical view.** PRD 120 F-120-24: no override, force, or
advanced control exists anywhere. A 409 renders the reason, never a generic
failure.

## Goal

Let the operator act, without any path around the safety gate.

## Scope

- Manual dose buttons with preset volumes
- Automation toggle with a **confirmation showing dose, daily cap, and cooldown**
- Lockout clear, shown only when `clearable: true`
- **409 rendered as a specific refusal with what will clear it**
- Command status polled after submission
- **No override control of any kind**

## Non-goals

- Any bypass — architecturally forbidden.

## Dependencies

- M12-004

## Implementation notes

The confirmation before enabling automation is the moment a person hands a
pump to a program; showing the limits at that moment is worth the extra click.

Hiding the clear button when `clearable: false` prevents an operator repeatedly
attempting something that cannot work — a leak must physically resolve first.

Add a test that greps the UI source for override-shaped parameters.

## Acceptance criteria

- [ ] Manual dosing works and shows the delivered result.
- [ ] **A 409 renders the specific reason and what will clear it.**
- [ ] A refusal is presented as the system working, not as an error.
- [ ] The automation toggle shows dose, cap, and cooldown before enabling.
- [ ] The clear button is absent when `clearable: false`.
- [ ] **No override, force, or advanced control exists**, asserted by a source scan.

## Verification

```bash
cd ui/rhizo-ui && cargo test actions::
grep -rn 'force\|override\|bypass' ui/rhizo-ui/src/   # expect none
```

## Tests required

- Refusal rendering.
- Confirmation flow.
- **Source scan for override controls.**

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/actions.rs
```
