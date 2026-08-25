# Issue M13-009 — Add systemd deployment for the Raspberry Pi

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-008

## Context

Deployment-model section 2: systemd rather than Docker on a Pi — Docker adds
an I/O layer over an SD card for no benefit at this scale, and systemd gives
better restart semantics and journal integration.

## Goal

Make the home deployment supportable.

## Scope

- systemd units for `mosquitto` and `edge-controller`
- Restart policies distinguishing clean exit from panic
- `EnvironmentFile=` at mode 0600 for secrets
- Journald log retention limits
- A documented Pi installation procedure
- **A documented recommendation to place the database off the SD card**

## Non-goals

- Packaging as a .deb.

## Dependencies

- M13-008

## Implementation notes

The restart policy must respect the exit-code distinction from M3-001:
SIGTERM exits 0 (do not restart), a panic exits non-zero (restart). A blanket
`Restart=always` would mask a crash loop.

The off-SD-card recommendation is the single most valuable line in the
installation documentation.

## Acceptance criteria

- [ ] Units start both services on boot.
- [ ] The restart policy distinguishes clean exit from panic.
- [ ] Secrets come from a mode-0600 environment file.
- [ ] Journald retention is bounded.
- [ ] The system survives a reboot and resumes automatically.
- [ ] The installation procedure is documented and has been followed on a real Pi.
- [ ] The off-SD-card recommendation is documented.

## Verification

```bash
sudo systemctl status rhizo-edge
sudo reboot   # then verify recovery
```

## Tests required

- Manual: install on a Pi, reboot, verify recovery.

## Documentation impact

- Raspberry Pi installation procedure.

## Files likely affected

```text
deploy/systemd/rhizo-edge.service
deploy/systemd/mosquitto.service
docs/deployment/raspberry-pi.md
```
