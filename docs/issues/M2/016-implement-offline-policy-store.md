# Issue M2-016 — Implement offline policy persistence and atomic activation

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-008, M2-015

## Context

[ADR-015](../../adr/015-device-offline-autonomy.md) §7 requires
validate → stage → verify → activate → acknowledge. The simulator models NVS so
that SAFETY-019 can be tested long before firmware exists.

## Goal

Persist and activate offline policies exactly as the firmware must.

## Scope

- Subscribe to the retained `policy` topic
- Validate against declared capabilities and the shared hard limits
- Stage to a separate region with a checksum, verify read-back, then activate atomically
- Ignore a policy whose `policy_version` is <= the applied version
- Report `applied_policy_versions` in status
- A `--fault policy-interrupt:<step>` injection that kills the process at a chosen step

## Non-goals

- Evaluating the policy or scheduling autonomous doses (M6-019).

## Dependencies

- M2-008
- M2-015

## Implementation notes

The state file already mirrors NVS (M2-008); add `policy_active`,
`policy_staging`, and their checksums. Writes must be atomic (temp file plus
rename), otherwise the interrupt fault exercises a torn-file path instead of the
activation path it is meant to test.

Rejection must be **non-destructive**: an invalid policy leaves the previous one
active and reports the rejection. This is the property SCEN-095 checks at every
interruption point, so the step boundaries need to be real and individually
interruptible.

## Acceptance criteria

- [ ] A valid policy is staged, verified, activated, and acknowledged.
- [ ] An invalid policy is rejected and the previous policy stays active.
- [ ] A policy with a lower or equal version is ignored.
- [ ] Interruption at every step leaves **exactly one** valid active policy.
- [ ] A corrupt stored policy is refused at load and no default is substituted.
- [ ] A policy naming an undeclared actuator is rejected.
- [ ] A fresh subscriber receives the policy retained, completing the mqtt-v1
      positive-retention set with M2-010's `status` and `config` assertions.

## Verification

```bash
cargo test -p device-simulator policy::
cargo test safety_019
cargo test -p device-simulator --test integration retained_policy
```

## Tests required

- Each validation rejection.
- Interruption at every step (SCEN-095).
- Version monotonicity.
- Corrupt-store refusal (SCEN-094).

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/policy.rs
crates/device-simulator/src/state.rs
```
