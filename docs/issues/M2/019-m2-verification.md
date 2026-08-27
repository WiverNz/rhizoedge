# Issue M2-019 — M2 verification and exit criteria

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001, M2-002, M2-003, M2-004, M2-005, M2-006, M2-007, M2-008, M2-009, M2-010, M2-011, M2-012, M2-013, M2-014, M2-015, M2-016, M2-017, M2-018

## Context

Final gate for M2. The simulator's fidelity is the foundation of every
safety claim made in M3-M8, so this verification carries unusual weight.

## Goal

Verify every PRD 020 acceptance criterion, especially the permissiveness parity requirement.

## Scope

- Full gate plus simulator integration tests
- Verify the single-call-site property for `validate_water_command`
- Verify the simulator contains no offline evaluator or autonomous-dose scheduler
- Verify the normative MQTT retention rules through M2-010: `status`, `config`,
  and `policy` retained; `time`, `telemetry`, `actuator`, `events`, and all
  `commands/*` never retained
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
- M2-007
- M2-008
- M2-009
- M2-010
- M2-011
- M2-012
- M2-013
- M2-014
- M2-015
- M2-016
- M2-017
- M2-018

## Implementation notes

The verification that matters most: publish `requested_ml: 10000` directly to
the broker, bypassing any edge, and confirm the simulator does not deliver it.
Everything M6 claims about SAFETY-007 rests on this being true.

Also confirm by inspection that there is exactly one call site of
`validate_water_command` and no code path to actuation that avoids it.
An enabled stored offline policy must remain inert in M2: policy evaluation and
autonomous scheduling activate together in M6-019 through the shared crate.

## Acceptance criteria

- [x] All gate commands pass.
- [x] `docker compose up mosquitto device-simulator` runs standalone and telemetry is visible.
- [x] `safety_007_simulator_refuses_like_hardware` passes.
- [x] Exactly one call site of `validate_water_command`.
- [x] No `evaluate_offline` implementation/call site and no autonomous-dose scheduler exists in M2.
- [x] Isolation continues sampling and buffering while an enabled policy remains non-actuating.
- [x] The complete mqtt-v1 retention matrix passes: `status`, `config`, and
      `policy` retained; `time`, `telemetry`, `actuator`, `events`, and every
      `commands/*` topic not retained. The `time` assertion is non-vacuous.
- [x] ACL isolation holds.
- [x] A full cycle completes in under 10 s at scale 600.
- [x] `--fault restart-mid-dose` yields `interrupted` with `delivered_ml: null`.
- [x] ROADMAP.md updated and the report recorded.

## Verification

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RHIZO_REQUIRE_BROKER=1 cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy      --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run -p rhizo-docscheck

cargo test -p device-simulator --test safety_007
cargo test -p device-simulator --test integration retained_topics
cargo test -p device-simulator --test single_actuation_path
docker compose -f deploy/docker-compose.yml --profile devices up -d mosquitto device-simulator
```

All green. Evidence is recorded in [docs/reports/M2.md](../../reports/M2.md);
`RHIZO_REQUIRE_BROKER=1` turns the local-development skip into a failure, so the
broker-backed suites cannot pass by skipping.

The `docker compose` line gained `--profile devices`: the simulator is started
for a scenario and stopped after it, while the broker is what a developer leaves
running. The manual check confirmed a full `telemetry.batch` with six typed
samples on the broker, and a retained-only view containing exactly `status`,
`config`, and `policy`.

## Tests required

- Full suite; the automated oversized-command integration test is normative.
- An optional manual broker check may publish the checked-in oversized-command
  fixture, if one exists by M2-019; abbreviated pseudo-JSON is not acceptable.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
