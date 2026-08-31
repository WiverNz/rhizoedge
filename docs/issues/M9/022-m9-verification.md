# Issue M9-022 — M9 verification and exit criteria

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-001, M9-002, M9-003, M9-004, M9-005, M9-006, M9-007, M9-008, M9-009, M9-010, M9-011, M9-012, M9-013, M9-014, M9-015, M9-016, M9-017, M9-018, M9-019, M9-020, M9-021

## Context

Final gate for M9. Board-dependent criteria are marked so the milestone can
be substantially completed and reviewed before hardware arrives.

## Goal

Verify every PRD 090 acceptance criterion.

## Scope

- Host tests, compile verification, and the conformance test
- With a board: HIL-1 and HIL-2
- Update ADR-007, safety-invariants.md, ROADMAP.md; record the report

## Non-goals

- Real sensors or pump.

## Dependencies

- M9-001
- M9-002
- M9-003
- M9-004
- M9-005
- M9-006
- M9-007
- M9-008
- M9-009
- M9-010
- M9-011
- M9-012
- M9-013
- M9-014
- M9-015
- M9-016
- M9-017
- M9-018
- M9-019
- M9-020
- M9-021

## Implementation notes

**Board portability is verified structurally, not by owning two boards.** The
XIAO ESP32-C3 is not purchased, so nothing here waits on it. What is checked is
the seam: one board profile builds, zero or two fail loudly, no GPIO number
exists outside `src/board/`, and the `app/` host tests never mention a board.
If those hold, adding the XIAO is a file. If they do not, M9 shipped a
convention rather than a boundary, and the cost lands in M10–M14.

**The pending-result ledger is the one host-testable thing most likely to be
skipped.** It needs no board — fill it with fake adapters and the behaviour is
reachable in seconds — but it is invisible in normal operation, so it only ever
shows up in the field, during an edge outage, as a plant that was watered twice.
Do not accept "bounded, evicts oldest" as an answer inherited from the simulator
or from M9-017's event buffer; the reasoning for both is written out in M9-011
and neither transfers. If the M9 report cannot state what happens when the
ledger is full, M9 is not done.

**HIL-1 is the gate that matters.** Put a multimeter on the pump driver input
and confirm it never asserts across twenty resets, a watchdog reset, and ten
mid-boot power cuts. If the pump so much as twitches, the hardware pull-down is
wrong and no firmware correctness compensates.

Everything else in M9 can be verified on the host.

## Acceptance criteria

- [ ] `cargo build --release` succeeds for the ESP target with no board.
- [ ] The CI firmware job passes.
- [ ] ADR-007's toolchain section has been executed and corrected.
- [ ] ADR-007's firmware-structure section matches the board layer as built.
- [ ] `board-devkitm1` builds; zero or two board features fail with a clear
      `compile_error!` naming the available profiles.
- [ ] The structural board-isolation check passes: no GPIO number, pin polarity,
      or board-specific peripheral construction outside `src/board/`.
- [ ] The `app/` host tests are board-independent — they pass with no board
      profile selected, and their results do not vary by profile.
- [ ] CI builds every declared board profile from the same application code.
- [ ] Host tests cover boot safety, interrupted dose, dedup ring, and command validation.
- [ ] **The pending-result ledger's saturation behaviour is decided, documented,
      and tested** (M9-011): the ledger fails closed when full, no unacknowledged
      watering result is silently discarded in a way that can under-count
      delivered water, saturation is visible as a durable fault, the state
      survives a reboot at the boundary, and acknowledgement restores normal
      operation without loss or double-counting. If any eviction of an
      unacknowledged result was adopted, the report argues its safety
      equivalence explicitly.
- [ ] PRD 090 Open question 5 is **resolved**, with the chosen capacity and
      behaviour recorded there and in ADR-014.
- [ ] The conformance test shows identical behaviour to the simulator.
- [ ] **With a board:** it connects, appears online, publishes telemetry, applies config.
- [ ] **With a board:** a duplicate `command_id` across a power cycle is deduplicated.
- [ ] **With a board:** an oversized command is clamped or rejected.
- [ ] **With a board:** HIL-1 passes on a multimeter.
- [ ] **With a board:** blocking the `time` topic (or disconnecting the edge) refuses commands with `clock_unsynced` while telemetry continues.
- [ ] Documentation updated; report recorded.

## Verification

```bash
cd firmware/esp32-node && cargo build --release && cargo test
cargo test board_isolation
grep -rnE "(Gpio|gpio)[0-9]+" src/ --include=*.rs | grep -v "^src/board/"   # expect nothing
espflash flash target/riscv32imc-esp-espidf/release/esp32-node --monitor
```

## Tests required

- Host suite, conformance, and the HIL-1/HIL-2 checklists.

## Documentation impact

- ADR-007.
- [ADR-014](../../adr/014-failure-and-retry-policy.md) §Device-side
  pending-result ledger — record the saturation decision.
- [PRD 090](../../prd/090-esp32-rust-firmware.md) Open question 5 — mark it
  resolved.
- safety-invariants.md.
- ROADMAP.md.
- hil-runs record.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
docs/testing/hil-runs/
```
