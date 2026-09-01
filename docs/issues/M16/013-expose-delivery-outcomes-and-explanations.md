# Issue M16-013 — Expose delivery outcomes and explanations over the API

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-012

## Context

"Did my plant actually receive water?" is the question this whole milestone
exists to answer, and the answer has to be more than yes. Three genuinely
different situations must be three different answers in the **domain model**,
not three sentences a UI happens to choose between:

```text
verified 38.7 ml, measured by the reservoir scale
actuation confirmed, but no witness is fitted on this node
unknown — the device restarted during actuation
```

The recommendation engine established the discipline: typed reasons, prose in
exactly one place, and a "no" as explainable as a "yes".

## Goal

The watering history and attempt endpoints, carrying every distinction the model
records.

## Scope

- `GET /api/v1/plants/{id}/waterings` gains `outcome`, `evidence_level`,
  `measured_ml`, and `verification` per row.
- `GET /api/v1/plants/{id}/waterings/{command_id}` returns the full attempt: all
  six doses, evidence level, outcome and typed reason, actuator start and stop,
  duration, settle result, witness health, calibration version and date, firmware
  version, reconciliation status, and `credited_ml`.
- `GET /api/v1/devices/{id}/actuators` returns health, witness state, and
  calibration age.
- Typed reason rendering in the one existing prose site.
- The six doses presented as a **ladder**, in order, with the step that changed
  each value named.

## Non-goals

- A UI. [PRD 120](../../prd/120-rust-ui.md) builds the screen; this makes it
  possible.
- Any endpoint that can cause an actuation.
- Free-prose fields in the domain model.

## Dependencies

- M16-012

## Implementation notes

The ladder is the useful part and it is easy to flatten by accident. Requested
50, authorised 30, commanded 30, effective 30, estimated 30, measured 28.4 tells
an operator that *the safety gate* cost them 20 ml and *the hardware* cost them
1.6 — two completely different conversations. A response that reports only
"requested 50, delivered 28.4" tells them neither and invites the wrong one.

`authorized_ml` and `commanded_ml` are equal in V1. Render both anyway. The
moment they can differ is the moment nobody will remember they could, and a
field that appears later is a client change.

Every outcome and reason is a typed value with a stable code, and prose is
produced only in the API layer — the property M5 established and the one most
likely to be eroded by an issue whose subject is explanation. A `String` reason
added here would be the first one in the project.

`verification` must never overstate. A row with no witness says
`evidence_level: "actuated"` and `outcome: "delivered_unverified"`, and the
absence of `measured_ml` is `null`, never `0`. This is the same rule the
telemetry contract already applies to an uncalibrated probe.

## Acceptance criteria

- [ ] The attempt endpoint returns all six doses, in order, each labelled.
- [ ] `authorized_ml` and `commanded_ml` are separate fields.
- [ ] A no-witness attempt reports `actuated` / `delivered_unverified` with
      `measured_ml: null`.
- [ ] An unknown attempt reports its typed reason.
- [ ] Every outcome and reason has a stable code, and none carries free prose.
- [ ] The three example answers in PRD 160 are each reproducible from one
      response body.
- [ ] A plant with no actuator still answers coherently (SAFETY-018).
- [ ] No endpoint added here can cause an actuation.

## Verification

```bash
cargo test -p edge-controller api::waterings
cargo test -p edge-controller delivery::explanation
curl -s localhost:8080/api/v1/plants/monstera-01/waterings | jq '.[0]'
curl -s localhost:8080/api/v1/plants/monstera-01/waterings/018fd7b1-… | jq
```

## Tests required

- Fixture-driven reproduction of each of the three example answers.
- Every outcome renders as JSON and as prose.
- `measured_ml` is `null`, never `0`, when absent.
- Response shape stable across a restart.

## Documentation impact

- `docs/protocol/http-api-boundaries.md`: the watering history and attempt
  endpoints, and the statement that neither can actuate.
- PRD 160 §API representation, if a field deviates.

## Files likely affected

```text
crates/edge-controller/src/api/waterings.rs
crates/edge-controller/src/api/actuators.rs
crates/edge-controller/src/delivery/explain.rs
docs/protocol/http-api-boundaries.md
```
