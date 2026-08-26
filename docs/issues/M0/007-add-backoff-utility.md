# Issue M0-007 — Implement exponential backoff with full jitter

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002

## Context

ADR-014 specifies one backoff implementation used by all five retry sites
(MQTT connect, MQTT publish, SQLite busy, cloud sync, device Wi-Fi). Full
jitter, not exponential-plus-a-little-random, because the naive form preserves
the synchronised retry storm that a broker restart causes.

## Goal

Provide the single shared backoff utility with tested bounds.

## Scope

- `Backoff { base, cap, attempt }` with `next_delay()` and `reset()`
- `delay = random_uniform(0, min(cap, base * 2^attempt))`
- Overflow-safe exponentiation for large attempt counts
- A seedable RNG hook so tests are deterministic

## Non-goals

- Any retry loop that uses it — each site wires it up itself.

## Dependencies

- M0-002

## Implementation notes

The exponent must saturate rather than overflow: at attempt 64,
`base * 2^attempt` overflows `u64` nanoseconds. Clamp the shift before
multiplying.

Full jitter means the delay can be very short, which is correct and
occasionally surprising — document it so nobody 'fixes' it into a minimum.

## Acceptance criteria

- [x] `next_delay()` never exceeds `cap`.
- [x] `next_delay()` is always non-negative.
- [x] Delay distribution widens with attempt count up to the cap.
- [x] `reset()` returns to the first attempt.
- [x] 1000 attempts do not overflow or panic.
- [x] With a seeded RNG, the sequence is reproducible.

## Verification

```bash
cargo test -p rhizo-telemetry backoff::
PROPTEST_CASES=10000 cargo test -p rhizo-telemetry backoff
```

## Tests required

- Property: delay in `[0, min(cap, base*2^n)]` for random n.
- Cap respected at high attempt counts.
- No overflow at attempt 1000.
- Reset behaviour.
- Seeded reproducibility.

## Documentation impact

- Doc comment explaining why full jitter, with a pointer to ADR-014.

## Files likely affected

```text
crates/telemetry/src/backoff.rs
```
