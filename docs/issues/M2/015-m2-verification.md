# Issue M2-015 — M2 verification and exit criteria

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001, M2-002, M2-003, M2-004, M2-005, M2-006, M2-008, M2-007, M2-009, M2-010, M2-011, M2-012, M2-013, M2-014

## Context

Final gate for M2. The simulator's fidelity is the foundation of every
safety claim made in M3-M8, so this verification carries unusual weight.

## Goal

Verify every PRD 020 acceptance criterion, especially the permissiveness parity requirement.

## Scope

- Full gate plus simulator integration tests
- Verify the single-call-site property for `validate_water_command`
- Verify no retained messages on command topics
- Verify ACL isolation
- Update ROADMAP.md and record the report

## Non-goals

- New behaviour.

## Dependencies

- M2-001
- M2-002
- M2-003
- M2-004
- M2-005
- M2-006
- M2-008
- M2-007
- M2-009
- M2-010
- M2-011
- M2-012
- M2-013
- M2-014

## Implementation notes

The verification that matters most: publish `requested_ml: 10000` directly to
the broker, bypassing any edge, and confirm the simulator does not deliver it.
Everything M6 claims about SAFETY-007 rests on this being true.

Also confirm by inspection that there is exactly one call site of
`validate_water_command` and no code path to actuation that avoids it.

## Acceptance criteria

- [ ] All gate commands pass.
- [ ] `docker compose up mosquitto device-simulator` runs standalone and telemetry is visible.
- [ ] `safety_007_simulator_refuses_like_hardware` passes.
- [ ] Exactly one call site of `validate_water_command`.
- [ ] No retained messages on command or telemetry topics.
- [ ] ACL isolation holds.
- [ ] A full cycle completes in under 10 s at scale 600.
- [ ] `--fault restart-mid-dose` yields `interrupted` with `delivered_ml: null`.
- [ ] ROADMAP.md updated and the report recorded.

## Verification

```bash
cargo test --workspace --all-features
cargo test safety_
docker compose up -d mosquitto device-simulator
mosquitto_pub -h localhost -u rhizo-edge -P "$P" -t 'rhizo/v1/devices/plant-node-01/commands/water' -q 1 -m '{...requested_ml:10000...}'
# assert the simulator clamps or rejects
```

## Tests required

- Full suite plus the manual oversized-command check.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
