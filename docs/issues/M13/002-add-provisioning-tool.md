# Issue M13-002 — Add the rhizo-provision tool

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001

## Context

ADR-012: per-device credentials mean per-device provisioning. Ten devices by
hand is ten opportunities to reuse a password.

## Goal

Make adding a device a single command.

## Scope

- `rhizo-provision new` generating a `device_id` and 32-byte password
- Updating the Mosquitto password file and triggering a reload
- Emitting ready-to-paste serial provisioning commands
- **Refusing to overwrite an existing device without `--force`**
- `list`, `revoke`, `rotate`
- **Never reusing a password**

## Non-goals

- Over-the-air provisioning.

## Dependencies

- M13-001

## Implementation notes

The overwrite refusal prevents the most likely operator error: re-running
`new` for an existing device and silently invalidating its credentials while it
is deployed in a pot on a high shelf.

`revoke` marks the device retired rather than deleting it, so history stays
attributable.

## Acceptance criteria

- [ ] `new` produces working credentials in one command.
- [ ] The broker reloads without a restart.
- [ ] **Re-running for an existing device is refused without `--force`.**
- [ ] Passwords are never reused.
- [ ] `revoke` removes credentials and marks the device retired.
- [ ] `rotate` works on a deployed device.
- [ ] `list` shows devices without exposing passwords.

## Verification

```bash
cargo run -p rhizo-provision -- new --name bedroom-node
cargo run -p rhizo-provision -- list
```

## Tests required

- Credential generation and uniqueness.
- **Overwrite refusal.**
- Revoke marks retired, does not delete.

## Documentation impact

- Provisioning procedure.

## Files likely affected

```text
crates/provision/src/main.rs
scripts/gen-mosquitto-passwd.sh
```
