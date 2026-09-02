# Issue M9-004 — Implement NVS storage and device identity

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-003

## Context

ADR-012: identity derived from MAC at first boot, stored in NVS, overridable
by serial provisioning — so one firmware image serves every device.

## Goal

Implement persistent device state and identity.

## Scope

- `NvsStore` trait with an ESP-IDF impl and a host fake
- The NVS layout from PRD 090
- `device_id` from NVS, or derived as `plant-node-<3-byte MAC hex>`
- `boot_id` fresh each boot
- Corrupt NVS: start with defaults, log, publish `nvs_reset`
- The 16-entry command dedup ring persisted

## Non-goals

- Serial provisioning (M9-006).

## Dependencies

- M9-003

## Implementation notes

Corrupt NVS must not block boot but must not be trusted either — the same
posture as the simulator's state file (M2-007).

The dedup ring in NVS is what makes SAFETY-001 survive a power cycle: a
`command_id` executed before a reboot must still be recognised after it.

Writing NVS on every dose is a flash-wear consideration; the ring is small and
doses are infrequent, so it is acceptable — note it.

## Acceptance criteria

- [ ] `device_id` is derived from MAC on first boot and persisted.
- [x] An NVS override wins over the derived value.
- [x] `boot_id` changes each boot.
- [x] Corrupt NVS starts fresh with a log and an event.
- [x] The dedup ring persists across a power cycle.
- [x] Host tests cover all of this with the fake store.

## Verification

```bash
cd firmware/esp32-node && cargo test nvs:: identity::
```

## Tests required

- Derivation.
- Override precedence.
- Corrupt recovery.
- Ring persistence.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/nvs.rs
firmware/esp32-node/src/app/identity.rs
```
