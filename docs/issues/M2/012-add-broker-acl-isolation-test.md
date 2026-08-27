# Issue M2-012 — Test broker ACL isolation between devices

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-002, M0-008

## Context

ADR-012 makes the Mosquitto `%u` ACL pattern the mechanism that turns device
identity into a real boundary. ADR-002 lists ACL misconfiguration silently
granting broad access as a risk. An untested ACL is an assumption.

## Goal

Prove a device cannot publish into another device's topic subtree.

## Scope

- An integration test with two authenticated device accounts
- Assert each can publish to its own subtree
- Assert each is **denied** publishing to the other's
- Assert anonymous connection is refused
- Assert the edge account can subscribe broadly

## Non-goals

- TLS or certificates (post-V1).

## Dependencies

- M2-002
- M0-008

## Implementation notes

Mosquitto's response to an ACL denial on publish is to drop the message
silently rather than to disconnect, so the assertion must be that a subscriber
never receives it — not that the publish call errored. Getting this wrong
produces a test that passes regardless.

This is SCEN-016.

## Acceptance criteria

- [x] `plant-node-01` publishes successfully to its own topic.
- [x] A message published by `plant-node-01` to `plant-node-02`'s topic is **never received** by a subscriber.
- [x] Anonymous connection is refused.
- [x] The `rhizo-edge` account subscribes to `rhizo/v1/devices/+/#` successfully.
- [x] The test fails if the ACL file is emptied.

## Verification

```bash
cargo test -p device-simulator --test integration -- acl_isolation_between_devices a_device_cannot_subscribe
```

Two negative controls were run and reverted:

- **ACL file emptied** — both tests fail. Mosquitto with no rules denies
  everything, so they fail on the *positive* assertions ("a device must be able
  to publish into its own subtree").
- **ACL pattern widened to `rhizo/v1/#`** — the sharper control, and the
  misconfiguration ADR-002 actually warns about. Here the positives pass and the
  isolation assertions fail precisely: *"a device published into another
  device's subtree and it was delivered"* and *"plant-node-01 received
  plant-node-02's traffic"*.

`.github/workflows/ci.yml` gains a `broker` job that generates throwaway
credentials, starts Mosquitto, and runs the broker-backed suites with
`RHIZO_REQUIRE_BROKER=1` — which turns the local-development skip into a
failure, so CI cannot pass by skipping its own subject.

## Tests required

- SCEN-016.
- Negative: empty the ACL file, confirm the test fails, revert.

## Documentation impact

- `.github/workflows/ci.yml`: a `broker` job, so the broker-backed tests run
  rather than skip.

## Files likely affected

```text
crates/device-simulator/tests/integration.rs
deploy/mosquitto/aclfile
```
