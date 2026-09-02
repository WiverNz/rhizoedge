# Issue M9-016 — Integrate the offline evaluator

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-015, M9-011

## Context

The firmware calls `rhizo_policy::evaluate_offline` — the same function the
simulator calls and the edge validates with. One implementation, one call site
([ADR-008](../../adr/008-shared-code-simulator-and-firmware.md)).

## Goal

Run the offline evaluator on the device and actuate through the existing gate.

## Scope

- Detect isolation (MQTT/Wi-Fi unavailable) and switch to offline evaluation
- Feed locally sampled measurements, leak, tank, pump health, and the persisted state
- Pass **monotonic** elapsed time as the `elapsed` parameter
- Actuate through the existing `validate_water_command` path — no second route
- Buffer an audit event for every dose and every refusal
- Return to edge control immediately on reconnection

## Non-goals

- A second implementation of any offline rule.
- Buffering mechanics (M9-017).

## Dependencies

- M9-015
- M9-011

## Implementation notes

There must remain **exactly one** actuation call site in the firmware. Offline
dosing routes into the same gate as commands, so the hard limits, leak veto, tank
veto, and pump-fault veto apply identically. Verify with a grep-based test, the
same way M2 does.

Use the monotonic timer, never the wall clock. `evaluate_offline` cannot read a
clock, so the only way to get this wrong is at the call site — which is exactly
where the test should look.

On reconnection, hand control back to the edge promptly but do **not** discard
buffered events; M9-017 owns their lifecycle.

## Acceptance criteria

- [x] An isolated device with a valid enabled policy waters within bounds.
- [x] An isolated device with no policy never waters.
- [x] Elapsed time comes from the monotonic timer, not the wall clock.
- [x] `grep -c validate_water_command` shows exactly one call site.
- [x] Every refusal is buffered as an audit event with its reason.
- [ ] Control returns to the edge on reconnection.
- [x] Host tests cover all of the above with fake adapters.

## Verification

```bash
cd firmware/esp32-node && cargo test offline::
cargo test safety_013 safety_017
grep -rn 'validate_water_command' firmware/esp32-node/src | wc -l
```

## Tests required

- Isolation switchover.
- No-policy refusal.
- Monotonic source assertion.
- Single-call-site assertion.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/app/offline.rs
```
