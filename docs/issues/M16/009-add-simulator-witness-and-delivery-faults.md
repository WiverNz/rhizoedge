# Issue M16-009 — Add the simulator witness and delivery faults

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-008

## Context

Every safety property this project claims is provable without hardware first,
and then re-verified with it. The simulator is what makes that true: M2-013's
fault set is why `pump-stuck-on` could be tested before a relay existed, and why
M11-002's independent run guard had a specification to match rather than to
invent.

The delivery faults need the same treatment, and they need it before M16-015
puts water on a bench.

## Goal

A simulator delivery witness and the fault set, so every PRD 160 scenario runs
in CI with no hardware.

## Scope

- A witness in the simulator's environment model: reservoir mass, dose
  depletion, noise, and a settling model.
- Faults: `no-flow`, `slow-flow-start`, `partial-flow`, `over-delivery`,
  `residual-flow`, `unauthorised-flow`, `witness-unreadable`,
  `witness-nonmonotonic`, `witness-implausible`, and `witness-absent`.
- The existing `disconnect-mid-dose` and `restart-mid-dose` faults extended to
  cover the baseline lifecycle.
- The `delivery` object on simulated results.
- `--witness reservoir|none` on the CLI, defaulting to `none`.

## Non-goals

- Modelling real hydraulics. The simulator's job is to exercise the protocol and
  the state machine, not to be a fluid simulation.
- Changing any existing simulator default. A run without `--witness` must behave
  exactly as it does today.

## Dependencies

- M16-008

## Implementation notes

`--witness none` is the default deliberately: every existing scenario, fixture,
and integration test must produce byte-identical behaviour after this issue. If
anything changes without the flag, the additive claim in M16-003 is not true and
this is where that gets discovered.

The simulator waters through the same `begin_dose` and the same shared
`validate_water_command` it does today, and this issue must not add a second
path. The witness is an observer of the dose, not a participant in authorising
it — the same relationship the real one has.

`unauthorised-flow` is the one fault with no dose at all: it depletes the
reservoir while idle, which is what a siphon does. It exists because M16-008's
idle detector is otherwise untestable, and because that is the failure with the
largest downside and the least existing coverage.

The `sleeping` simulator disconnects **cleanly** so the broker does not publish
the will and overwrite the retained sleep status. A witness fault raised while
sleeping must follow the same discipline; the real-broker test is what caught
this the first time.

## Acceptance criteria

- [ ] Without `--witness`, every existing test and scenario is unchanged.
- [ ] With `--witness reservoir`, a normal dose produces a `delivery` object and
      `delivered_verified`.
- [ ] Every fault in scope produces its documented outcome.
- [ ] `unauthorised-flow` raises the fault with no dose in flight.
- [ ] `restart-mid-dose` exercises both the valid-baseline and lost-baseline
      paths.
- [ ] `tests/single_actuation_path.rs` still passes: no second actuation path.
- [ ] Faults work against a real broker, not only in process.

## Verification

```bash
cargo test -p rhizo-device-simulator delivery::
cargo test -p rhizo-device-simulator single_actuation_path
RHIZO_REQUIRE_BROKER=1 cargo test -p rhizo-device-simulator --all-features
```

## Tests required

- Each fault, in process and against a broker.
- Default-off equivalence with pre-M16 behaviour.
- The clean-disconnect discipline for a witness fault while sleeping.

## Documentation impact

- `docs/testing/simulator-strategy.md`: the witness and the new faults.
- `docs/testing/failure-scenarios.md`: scenarios registered in M16-016.

## Files likely affected

```text
crates/device-simulator/src/environment.rs
crates/device-simulator/src/witness.rs
crates/device-simulator/src/fault.rs
crates/device-simulator/src/cli.rs
docs/testing/simulator-strategy.md
```
