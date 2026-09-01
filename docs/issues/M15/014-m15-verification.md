# Issue M15-014 — M15 verification

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-013

## Context

M15's exit criteria are unusual in one respect: most of them are claims about
what did **not** change. A milestone that adds inference to a safety system is
judged by whether the safety system is still the same safety system, and that
has to be demonstrated rather than asserted.

## Goal

Demonstrate every M15 exit criterion end to end, register the scenarios, settle
the starting-value constants against real data, and write the milestone report.

## Scope

- End-to-end scenarios in the M8 environment, registered in
  `docs/testing/failure-scenarios.md` with identifiers allocated at this point:
  cold start, learning to confidence, an adaptive dose clamped at the static
  ceiling, an adaptive dose refused by the rolling cap, a repot mid-learning, a
  sensor replacement mid-learning, contamination by manual watering, an edge
  restart mid-learning, a stale-reading fallback, and a corrupt-model fallback.
- The replay proof: for every scenario plant, `rebuild` from the observation
  ledger equals the incrementally maintained model, exactly.
- The invariance proof: with `adaptive_mode = disabled` on every plant, the M6,
  M8, and M13 suites produce byte-identical decisions to their pre-M15 baseline.
- Revisiting `MIN_SEGMENTS`, `MIN_RESPONSES`, `OBSERVATION_HALF_LIFE_DAYS`,
  `MAD_OUTLIER_K`, `EPOCH_STALE_DAYS`, and `MIN_EFFECTIVE_ML` against the real
  deployment's history, and recording the reasoning wherever a value moves.
- `docs/reports/M15.md`.

## Non-goals

- New behaviour of any kind.
- Enabling adaptive mode by default. It stays `disabled`, and turning it on is
  an operator decision on a plant they are watching.

## Dependencies

- M15-013

## Implementation notes

**Quote the environment and the per-suite counts, never a bare workspace total.**
Forty-six tests are broker-gated and count as passed whether or not a broker is
running, so the workspace total is identical either way. This is what the post-M6
pass corrected in the M6 report, and an inference milestone is exactly where a
misleading number would do the most damage.

The invariance proof is the most important artefact this issue produces, and it
needs a real baseline: capture the pre-M15 decision series for the scenario
plants **before** M15-012 merges, store it as a fixture, and compare. Rerunning
the suites and observing that they pass is not the same claim.

The constants are starting values and this is where they stop being guesses. A
value that moves needs its evidence in the report — how many plants, over how
long, and what went wrong at the old value. A value that stays needs a sentence
saying it was checked.

Run the full project gate, including both bare-metal builds: M15 claims to have
left `rhizo-mqtt-contract` and `rhizo-policy` untouched, and the cheapest way to
prove it is to build them.

## Acceptance criteria

- [ ] Every PRD 150 acceptance criterion is demonstrated, with evidence.
- [ ] Every scenario runs in the M8 environment with no hardware.
- [ ] Scenarios are registered in `failure-scenarios.md` with allocated IDs.
- [ ] `rebuild` equals the maintained model for every scenario plant.
- [ ] With adaptive disabled, decisions are byte-identical to the captured
      pre-M15 baseline.
- [ ] `cargo test safety_` passes, including every `safety_022_*` test.
- [ ] Both bare-metal targets build, and `rhizo-mqtt-contract` and
      `rhizo-policy` have no M15 diff.
- [ ] The ADR-005 catalogue is unchanged, and the catalogue tests still pass.
- [ ] `cargo run -p rhizo-docscheck` is clean.
- [ ] `docs/reports/M15.md` quotes the environment and per-suite counts.
- [ ] ROADMAP, CLAUDE.md, and the safety registry are updated in this change.

## Verification

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RHIZO_REQUIRE_BROKER=1 cargo test --workspace --all-features
cargo test safety_
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
git diff --stat <pre-M15-tag> -- crates/mqtt-contract crates/policy
docker compose -f deploy/docker-compose.yml config
cargo run -p rhizo-docscheck
```

## Tests required

- The full scenario suite.
- The replay proof, as a test rather than a manual step.
- The invariance proof against the captured baseline fixture.

## Documentation impact

- `docs/reports/M15.md` — new.
- `ROADMAP.md` — M15 status, and the §7 note that a deterministic per-plant
  estimator is not the machine learning that remains out of scope.
- `CLAUDE.md` §1 — the status table.
- `docs/architecture/safety-invariants.md` — SAFETY-022 enforced, with its tests.
- `docs/testing/failure-scenarios.md` — the new scenarios.

## Files likely affected

```text
docs/reports/M15.md
docs/testing/failure-scenarios.md
docs/architecture/safety-invariants.md
ROADMAP.md
CLAUDE.md
crates/scenarios/
```
