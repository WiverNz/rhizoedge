# Issue M13-011 — Add multi-device end-to-end scenarios

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-004, M13-007

## Context

The M8 scenarios assume one device. Multi-device failure modes — one device
dying while others continue, a shared reservoir emptying — need their own
coverage.

## Goal

Extend the scenario suite to multi-device operation.

## Scope

- 5 devices, 10 plants operating independently
- **Cross-plant isolation under every failure mode (SCEN-080)**
- Shared reservoir depletion locking out multiple plants
- Simultaneous commands to different devices
- Notification dedup under a storm
- Tick budget at 20 plants

## Non-goals

- Load testing beyond 20 plants.

## Dependencies

- M13-004
- M13-007

## Implementation notes

SCEN-080 is the one to get right: run a control scenario, then a variant
where plant A experiences every failure mode, and diff plant B's state history
between the two. Any difference is cross-plant interference.

## Acceptance criteria

- [ ] 5 devices and 10 plants operate independently in the suite.
- [ ] **SCEN-080 shows byte-identical state for unaffected plants.**
- [ ] Shared reservoir depletion locks out all dependent plants.
- [ ] Simultaneous commands to different devices succeed.
- [ ] A notification storm produces coalesced alerts.
- [ ] The tick budget holds at 20 plants.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml run --rm scenario-runner --scenario scenario_multi_device
```

## Tests required

- **SCEN-080.**
- Shared reservoir.
- Simultaneous commands.
- Tick budget.

## Documentation impact

- failure-scenarios.md extended.

## Files likely affected

```text
test/scenarios/multi_device.rs
```
