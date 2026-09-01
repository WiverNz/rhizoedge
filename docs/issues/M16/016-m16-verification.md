# Issue M16-016 — M16 verification

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-015

## Context

M16 makes two kinds of claim, and they are verified differently. The new claims
— a blocked tube is caught on the first dose, an unknown outcome is never zero —
are demonstrated by scenarios and tests. The claims about what did **not** change
— every existing safety property, every existing decision, every existing
message — are demonstrated by showing that a system with no witness fitted
behaves identically to the one that existed before.

The second kind is the one a milestone like this gets wrong.

## Goal

Demonstrate every exit criterion, register the scenarios, and write the report.

## Scope

- End-to-end scenarios in the M8 environment, registered in
  `docs/testing/failure-scenarios.md` with identifiers allocated here: verified
  delivery; under-delivery within and beyond tolerance; over-delivery; no flow;
  delayed startup inside and outside the timeout; clean settle; continued flow;
  unauthorised flow; tank empty mid-dose; leak mid-dose; witness absent; witness
  invalid; disconnect before actuation; disconnect during actuation; device
  restart mid-dose; edge restart mid-dose; duplicate result; duplicate
  `command_id`; TTL expiry with no result; and budget totals after each uncertain
  outcome.
- **The invariance proof**: with no witness configured, the M6, M8, and M11
  suites produce decisions byte-identical to a captured pre-M16 baseline.
- **The additive-wire proof**: every pre-M16 protocol fixture decodes and
  re-encodes unchanged, and both bare-metal targets build.
- `docs/reports/M16.md`.

## Non-goals

- New behaviour of any kind.
- Enabling a witness by default. `NullWitness` stays the default; fitting one is
  a hardware decision an operator makes.

## Dependencies

- M16-015

## Implementation notes

**Quote the environment and the per-suite counts, never a bare workspace total.**
Forty-six tests are broker-gated and count as passed whether or not a broker is
running, so the workspace total is identical either way. This is what the post-M6
pass corrected in the M6 report, and a milestone whose subject is "what actually
happened" is the worst possible place for a misleading number.

The invariance proof needs a real baseline, captured **before** M16-011 merges —
that is the issue that changes budget arithmetic, and after it the comparison is
no longer a comparison. Store the pre-M16 decision series as a fixture and diff
against it. Rerunning the suites and observing that they pass is a weaker claim
and not the one being made.

The additive-wire proof is cheap and load-bearing: an old fixture that decodes is
half of it, and an old fixture that **re-encodes without materialising a
`delivery` key** is the other half. Without the second, "additive" means "we
think so".

Run the full project gate including both bare-metal builds and a diff of
`crates/mqtt-contract` and `crates/policy` against the pre-M16 tag — M16 claims
the `no_std` crates changed only additively, and the cheapest proof is to build
them and read the diff.

## Acceptance criteria

- [ ] Every PRD 160 acceptance criterion is demonstrated, with evidence.
- [ ] Every scenario runs in the M8 environment with no hardware.
- [ ] Scenarios are registered in `failure-scenarios.md` with allocated IDs.
- [ ] With no witness, decisions are byte-identical to the captured pre-M16
      baseline.
- [ ] Every pre-M16 protocol fixture decodes **and** re-encodes unchanged.
- [ ] `cargo test safety_` passes, including every `safety_023_*` and
      `safety_024_*` test.
- [ ] Both bare-metal targets build.
- [ ] HIL-8's required gates are all recorded as passed.
- [ ] The ADR-005 catalogue is unchanged and its tests still pass.
- [ ] `cargo run -p rhizo-docscheck` is clean.
- [ ] `docs/reports/M16.md` quotes the environment and per-suite counts.
- [ ] ROADMAP, CLAUDE.md, and the safety registry are updated in this change.

## Verification

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RHIZO_REQUIRE_BROKER=1 cargo test --workspace --all-features
cargo test safety_
cargo test -p rhizo-mqtt-contract --test fixtures
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run -p rhizo-docscheck
```

## Tests required

- The full scenario suite.
- The invariance proof against the captured baseline fixture.
- The additive-wire proof, in both directions.

## Documentation impact

- `docs/reports/M16.md` — new.
- `ROADMAP.md` — M16 status.
- `CLAUDE.md` §1 — the status table.
- `docs/architecture/safety-invariants.md` — SAFETY-023 and SAFETY-024 enforced,
  with their tests.
- `docs/testing/failure-scenarios.md` — the new scenarios.

## Files likely affected

```text
docs/reports/M16.md
docs/testing/failure-scenarios.md
docs/architecture/safety-invariants.md
ROADMAP.md
CLAUDE.md
crates/scenarios/
```
