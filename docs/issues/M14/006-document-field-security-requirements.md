# Issue M14-006 — Document field deployment security requirements

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-002

## Context

PRD 140: a field gateway on a public network needs TLS, per-device
certificates, signed firmware, and authenticated APIs. **This is the largest
single gap between V1 and any field deployment** and must not be treated as an
increment.

## Goal

State the security gap honestly and specify what closing it requires.

## Scope

- V1's actual posture: no TLS, no API auth, no certificates, no signed firmware
- What a public-network deployment requires
- The `device_id`-as-username to certificate-CN migration path
- Signed firmware and secure boot requirements
- Cloud authentication
- **An explicit statement that V1 must not be exposed to an untrusted network**

## Non-goals

- Implementing any of it.

## Dependencies

- M14-002

## Implementation notes

State the current posture plainly rather than softening it: anyone on the LAN
who can reach port 8080 can water a plant, and MQTT credentials cross the network
in the clear. That is acceptable for a trusted home network and unacceptable for
anything else.

The ACL model survives the certificate transition unchanged, since `device_id`
maps directly onto a CN — that is the one piece of good news and worth
recording.

## Acceptance criteria

- [ ] V1's posture is stated plainly and completely.
- [ ] Field requirements are specified.
- [ ] The certificate migration path is described, including the surviving ACL model.
- [ ] Signed firmware requirements are outlined.
- [ ] **An explicit warning against exposing V1 to an untrusted network is recorded.**
- [ ] The gap is characterised as a project, not an increment.

## Verification

```bash
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Review-based.

## Documentation impact

- docs/architecture/security-roadmap.md; README limitation note.

## Files likely affected

```text
docs/architecture/security-roadmap.md
README.md
```
