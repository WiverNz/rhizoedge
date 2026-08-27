# MQTT v1 fixtures

These files are append-only compatibility artefacts. Once a valid v1 fixture is
checked in it must continue to decode for the lifetime of v1; correct the
implementation rather than rewriting history. Invalid fixtures document a
specific rejected shape.

The corpus is exercised by `crates/mqtt-contract/tests/fixtures.rs`, which
discovers every file from this directory tree. Adding a fixture for a message
kind or failure class that already exists needs **no Rust change**.

## `valid/`

Each file decodes into its **concrete payload type** — chosen from the envelope's
`kind` — and then runs that payload's own validation. Every JSON path present in
the file must survive a decode/re-encode round trip with its value intact. A type
may add a field the fixture omits (a `#[serde(default)]` such as `point`); it may
never lose or alter one the fixture states. That asymmetry is the whole point:
renaming a wire field turns this suite red instead of silently breaking a fleet.

Every message kind in protocol §3 must keep at least one example here.

## `invalid/`

Each fixture lives in a directory **named for the failure it proves**:

```text
invalid/<expected_variant>/<case>.json
```

The directory name maps to a case of `Expected` in the test, which asserts the
exact typed error — `DecodeError::UnsupportedVersion`,
`PolicyError::DoseAboveHardLimit`, `EventBatchError::DuplicateEventId`, and so
on. Asserting merely "it failed" would let a fixture keep passing for the wrong
reason.

A directory the test does not recognise is a **hard failure**, not a skip: an
unclassified fixture proves nothing, so the suite refuses to ignore it.

## History

The corpus was hardened before M2 began. The original M1 harness decoded valid
fixtures only as `Envelope<serde_json::Value>`, so it verified the envelope and
never the payload, and it enumerated invalid fixtures by hard-coded filename. A
few early fixtures were empty stubs (`"policies": []`, `"events": []`) that
exercised no payload structure; those were filled in during the same pass. That
is the one deliberate exception to the append-only rule, taken while no v1 device
existed. It does not apply again.
