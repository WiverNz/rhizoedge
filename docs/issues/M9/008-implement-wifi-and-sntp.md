# Issue M9-008 — Implement Wi-Fi connection and SNTP sync

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-004

## Context

SAFETY-002 depends on a synced wall clock: a device that cannot evaluate TTL
must refuse every water command. Wi-Fi robustness is the most important
non-safety behaviour in this firmware.

## Goal

Connect to Wi-Fi and synchronise time reliably.

## Scope

- Wi-Fi with credentials from NVS
- Reconnect with full-jitter backoff, base 2 s cap 300 s, unlimited
- SNTP sync after association
- `clock_synced` reported truthfully in status
- RSSI reported
- **Sampling continues while disconnected**

## Non-goals

- Provisioning the credentials (M9-006).

## Dependencies

- M9-004

## Implementation notes

An isolated device is still a monitoring device: keep sampling, keep the pump
off, and never water autonomously. The device has no irrigation logic by design.

Report `clock_synced` honestly. A device that claimed sync it did not have would
turn SAFETY-002 into a lie, and there is no way to detect it from the edge.

A LAN without outbound NTP needs a local NTP server — document that consequence.

## Acceptance criteria

- [ ] The device connects with NVS credentials.
- [ ] Reconnect uses the documented backoff.
- [ ] SNTP sync completes and `clock_synced` becomes true.
- [ ] Blocking SNTP leaves `clock_synced` false and it is reported as such.
- [ ] Sampling continues while Wi-Fi is down.
- [ ] The pump stays off while disconnected.
- [ ] RSSI is reported in status.

## Verification

```bash
cd firmware/esp32-node && cargo test net::
# with a board: observe connection and clock_synced in the edge API
```

## Tests required

- Host tests with a fake network layer.
- Backoff bounds.
- Sampling continues while disconnected.

## Documentation impact

- Deployment note about local NTP.

## Files likely affected

```text
firmware/esp32-node/src/net/wifi.rs
firmware/esp32-node/src/net/sntp.rs
```
