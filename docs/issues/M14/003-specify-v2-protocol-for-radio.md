# Issue M14-003 — Specify v2 protocol requirements for constrained radio links

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-002

## Context

PRD 140's design direction: a v2 protocol delivered as a gateway translation,
so the Edge Controller never learns about radios.

## Goal

Specify what v2 must provide, without implementing it.

## Scope

- Binary encoding requirements (CBOR or postcard) and expected payload sizes
- Envelope reduction: DevEUI implies `device_id`, shortened `message_id`
- **Command delivery to a sleeping device**: poll-on-wake versus downlink window
- **TTL semantics without a reliable clock** — the hardest problem
- Last Will replaced by an expected next-contact time
- The gateway translation boundary

## Non-goals

- Implementing v2.

## Dependencies

- M14-002

## Implementation notes

The TTL problem is the genuinely hard one and should be presented as such
rather than resolved prematurely. The V1 answer — refuse if unsynced — would
mean a battery node never waters. Options include a wake-count TTL, a
device-verified sequence horizon, or accepting monitoring-only field devices.

Monitoring-only field devices are a legitimate product and should be named as an
option, not treated as a failure.

## Acceptance criteria

- [ ] Payload size targets are computed for a representative message set.
- [ ] The envelope reduction is specified.
- [ ] Command delivery options are laid out with their trade-offs.
- [ ] **TTL alternatives are analysed, none prematurely chosen.**
- [ ] The Last Will replacement is specified.
- [ ] The gateway translation boundary is defined.
- [ ] Monitoring-only field devices are named as a legitimate option.

## Verification

```bash
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Review-based.

## Documentation impact

- docs/protocol/mqtt-v2-requirements.md.

## Files likely affected

```text
docs/protocol/mqtt-v2-requirements.md
```
