# Issue M0-010 — Create rhizo-testkit with a deterministic TestClock

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002

## Context

ADR-013 requires that domain logic never read the system clock, and that no
test sleeps to advance logical time. `TestClock` is the mechanism, and it must
exist before any time-dependent code is written.

## Goal

Provide the deterministic clock and the testkit crate that later fixtures extend.

## Scope

- `rhizo-testkit` crate
- `TestClock` with `new(at)`, `set(at)`, `advance(by)`, shared cheaply across tasks
- A `Clock` impl (the trait itself lands in M1-012)
- A documented rule that tests advance the clock rather than sleeping

## Non-goals

- Payload builders (M1).
- The MQTT spy (M2).
- Database fixtures (M3).

## Dependencies

- M0-002

## Implementation notes

`TestClock` must be `Clone + Send + Sync` and cheap to share — internally an
`Arc<Mutex<DateTime<Utc>>>` or an `Arc<AtomicI64>` of epoch millis. Tests will
hold one and hand clones to several components.

Since `rhizo-domain` does not exist yet, define the clock concretely here and
have it implement the trait in M1-012. Avoid inventing a second trait.

## Acceptance criteria

- [ ] `TestClock::new(t).now() == t`.
- [ ] `advance(d)` moves time forward by exactly `d`.
- [ ] `set` works backwards as well as forwards (clock-step tests need it).
- [ ] A clone observes the same time as the original.
- [ ] Concurrent reads from several tasks are consistent.

## Verification

```bash
cargo test -p rhizo-testkit
```

## Tests required

- Set/advance/read arithmetic.
- Clone shares state.
- Backwards set.
- Concurrent access from two tasks.

## Documentation impact

- Crate docs stating the no-sleep rule and pointing at ADR-013.

## Files likely affected

```text
crates/testkit/src/lib.rs
crates/testkit/src/clock.rs
```
