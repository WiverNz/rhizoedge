# Issue M12-015 — Implement connectivity and offline autonomy views

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-005, M12-004

## Context

[connectivity-modes.md](../../architecture/connectivity-modes.md) §7 requires the
operator to be told which mode a device is in and whether it is authorised to act
alone — never to infer it from silence.

## Goal

Surface connectivity mode and offline autonomy status honestly.

## Scope

- Device connectivity: connected, isolated (with duration), reconciling
- Per plant: offline automation enabled or disabled, policy version, applied version, drift
- The five presentations from connectivity-modes.md §7, verbatim in meaning
- Enable and disable offline autonomy, with a confirmation showing dose, budget, cooldown, and required sensors
- Show which measurements the offline policy requires and whether they are currently healthy

## Non-goals

- Offline history presentation (M12-016).

## Dependencies

- M12-005
- M12-004

## Implementation notes

The distinction the operator most needs is between "offline and monitoring only"
and "offline and watering itself". Those look identical if the UI only says
"offline", and they are completely different situations for someone deciding
whether to drive home.

Enabling offline autonomy deserves the same confirmation weight as enabling
connected automation, and arguably more: the operator is authorising a device to
water unsupervised with nobody watching. Show the bounds.

## Acceptance criteria

- [ ] All three connectivity states render distinctly.
- [ ] The five §7 presentations are implemented.
- [ ] Offline autonomy can be enabled and disabled, with a bounds confirmation.
- [ ] Policy version and applied version are shown, with drift flagged.
- [ ] Required measurements and their current health are visible.
- [ ] An isolated device shows how long it has been alone.

## Verification

```bash
cd ui/rhizo-ui && cargo test connectivity::
```

## Tests required

- Each connectivity state.
- Confirmation contents.
- Drift indication.

## Documentation impact

- connectivity-modes.md §7 verified.

## Files likely affected

```text
ui/rhizo-ui/src/views/connectivity.rs
```
