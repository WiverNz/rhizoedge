# Issue M9-006 — Add serial provisioning

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-004

## Context

ADR-012 prefers serial provisioning over build-time credentials, so one
firmware image serves every device and no binary contains a secret.

## Goal

Provision credentials over serial into NVS.

## Scope

- A serial command interface: `provision wifi|mqtt|device-id|show|commit`
- Writes to NVS; takes effect after reboot
- `show` redacts secrets
- Available only before network initialisation, or behind an explicit unlock
- Documented procedure

## Non-goals

- Over-the-air provisioning (post-V1).

## Dependencies

- M9-004

## Implementation notes

Restricting provisioning to the pre-network window (or an explicit unlock)
matters: a serial console reachable at runtime is a credential-disclosure path
on a device someone might place in a shared space.

`show` must redact — the whole point of serial provisioning is that credentials
never appear in a binary or a log.

## Acceptance criteria

- [ ] All provisioning commands work and persist to NVS.
- [ ] `show` redacts secrets.
- [ ] Settings take effect after reboot.
- [ ] Provisioning is unavailable at runtime without an explicit unlock.
- [ ] One firmware image works for multiple devices.
- [ ] The procedure is documented.

## Verification

```bash
espflash monitor  # then run the provisioning commands
```

## Tests required

- Host tests of the command parser and NVS writes.
- Redaction.

## Documentation impact

- Firmware provisioning procedure.

## Files likely affected

```text
firmware/esp32-node/src/provision.rs
```
