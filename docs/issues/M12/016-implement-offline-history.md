# Issue M12-016 — Implement offline history and gap presentation

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-015

## Context

After reconciliation the operator must be able to see what happened while nobody
was watching — including what was lost (SAFETY-020).

## Goal

Present reconciled offline history and gaps truthfully.

## Scope

- Watering history distinguishes `origin`: edge command versus offline autonomous
- Offline refusals with their reasons are visible
- **Gaps rendered as gaps** in charts — never interpolated across
- A gap shows its duration, event count, and tier
- An isolation period is shown as a band on the timeline
- Reconciliation status while in progress

## Non-goals

- Editing history — it is a ledger.

## Dependencies

- M12-015

## Implementation notes

Interpolating a chart across a gap is the failure mode to avoid. A smooth line
across four missing hours tells the operator the plant was fine when the truth is
that nobody knows, and that is exactly the period they most need to be suspicious
about.

Marking autonomous doses distinctly matters for trust: an operator who sees water
they did not authorise, with no indication of who authorised it, will not keep
using the feature.

## Acceptance criteria

- [ ] Autonomous doses are visually distinct from commanded ones.
- [ ] Offline refusals and their reasons are visible.
- [ ] **Charts render gaps as gaps**, with no interpolation.
- [ ] A gap shows duration, count, and tier.
- [ ] Isolation periods appear as timeline bands.
- [ ] Reconciliation in progress is shown rather than looking like a stall.

## Verification

```bash
cd ui/rhizo-ui && cargo test offline_history::
```

## Tests required

- Origin distinction.
- **Gap rendering without interpolation.**
- Isolation band rendering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/history.rs
ui/rhizo-ui/src/components/chart.rs
```
