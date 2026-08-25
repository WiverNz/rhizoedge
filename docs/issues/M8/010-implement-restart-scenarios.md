# Issue M8-010 — Implement the edge restart scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006

## Context

SCEN-051. Proves SAFETY-010 across real process boundaries rather than in a
simulated restart — which is where the interesting failures live.

## Goal

Prove restart safety end to end.

## Scope

- SCEN-051 kill the edge immediately after a command publish, restart, verify no replay
- Verify the late result is matched to the existing `command_id`
- Verify exactly one watering event and one daily-total contribution

## Non-goals

- Device restart (M9).

## Dependencies

- M8-006

## Implementation notes

Timing the kill precisely is the hard part. Use a fault hook in the edge that
exits immediately after the publish returns, rather than trying to race it
externally — an unreliable kill produces an unreliable test.

Assert via the MQTT spy that only one command was ever published across both
process lifetimes.

## Acceptance criteria

- [ ] The edge is killed reliably after publish, before the result.
- [ ] **No second command is published after restart.**
- [ ] The late result matches the existing `command_id`.
- [ ] Exactly one watering event exists.
- [ ] The daily total counts the dose once.
- [ ] Irrigation state including `wait_until` is restored exactly.

## Verification

```bash
... run --rm scenario-runner --scenario scenario_restart_mid_command
```

## Tests required

- SCEN-051.
- SCEN-052 restart mid-absorption.

## Documentation impact

- None.

## Files likely affected

```text
test/scenarios/restart.rs
```
