# Issue M14-008 — Design the future actuator capability model

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-004

## Context

[ADR-016](../../adr/016-plant-binding-and-policy-model.md) reserves actuator kinds
beyond `irrigation_pump` without implementing them. M14 checks that the reservation
is real and specifies what each would need.

## Goal

Verify the actuator expansion point and specify what future kinds require.

## Scope

- Verify `ActuatorBinding` and the actuator kind enum genuinely accommodate new kinds
- For `valve`, `grow_light`, `fan`, `heater`, `humidifier`, `fertiliser_dosing_pump`: what each needs in safety limits, state model, and policy
- **The valve-stuck-open failure**, which is worse than a stuck pump and needs a hardware-level bound
- Which future kinds would need a genuinely different automation model rather than an extension
- What a second actuator per plant would require

## Non-goals

- Implementing any actuator kind.
- Building a generic automation framework.

## Dependencies

- M14-004

## Implementation notes

The valve case deserves the most attention. A pump moves water only while
energised and stops when power is removed; a valve on a pressurised supply can
drain a reservoir or a mains line and has no natural duration bound. It needs a
hardware-level fail-closed bound independent of firmware — the field equivalent of
SAFETY-007 — and that is a design constraint, not an implementation detail.

Be honest about which kinds do not fit the current model. A grow light is a
schedule, not a dose, and modelling it as bounded actuation would distort both.
Saying so is more useful than forcing it into the existing shape.

## Acceptance criteria

- [ ] The expansion point is verified against the code, not just the ADR.
- [ ] Each future kind's requirements are specified.
- [ ] **The valve-stuck-open failure is analysed with its required hardware bound.**
- [ ] Kinds needing a different automation model are identified as such.
- [ ] Multi-actuator implications are described.
- [ ] **No implementation is added.**

## Verification

```bash
cargo run -p rhizo-docscheck
git diff --stat -- crates/ firmware/   # expect empty
```

## Tests required

- Review-based.

## Documentation impact

- ADR-016 actuator section.
- PRD 140.

## Files likely affected

```text
docs/architecture/zone-model.md
docs/prd/140-field-readiness.md
```
