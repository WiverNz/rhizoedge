# ADR-013 — Clock and time semantics

## Status

Accepted — 2026-08-25. Implemented across M1 (representation), M3 (stamping),
M6 (staleness, TTL, rolling window).

## Context

Three safety invariants are time-derived:

- SAFETY-002 — an expired command must not execute
- SAFETY-005 — a stale sample must not drive automatic watering
- SAFETY-006 — the rolling 24-hour cap must not be exceeded

There are three clocks in the system (edge, device wall, device monotonic), any
of which can be wrong, and the failure modes are not symmetric: a device clock
that is *behind* makes stale data look fresh, which is the dangerous direction.

The normative rules are in [time-model.md](../architecture/time-model.md). This
ADR records the reasoning and the alternatives.

## Decision

### The edge clock is authoritative for every safety computation

The edge stamps `received_at` on every ingested message from its own clock, and
staleness, the daily window, and command issue/expiry all use edge time.

The device timestamp (`device_time_ms`) is stored but **never** used to decide
freshness. If it were, a device whose clock was six hours behind would make
six-hour-old data appear current, defeating SAFETY-005 precisely when a
malfunctioning device most needs the lockout.

`device_time_ms` remains useful — for diagnosing drift, and for reconstructing
device-side ordering — so it is retained as advisory data.

### Representation: `INTEGER` Unix epoch milliseconds, UTC

| Layer | Form |
|---|---|
| SQLite / PostgreSQL storage | `INTEGER` ms (SQLite) / `TIMESTAMPTZ` (Postgres) |
| `rhizo-mqtt-contract` | `UtcMillis(i64)` |
| `rhizo-domain`, host code | `chrono::DateTime<Utc>` |
| MQTT JSON | integer ms |
| REST API JSON | RFC 3339 with `Z` |

Milliseconds over seconds because pump durations and message ordering need
sub-second resolution. Integer over ISO-8601 TEXT on the wire and in SQLite
because it indexes well, compares unambiguously, and cannot accidentally carry a
local-time offset. RFC 3339 in the REST API because humans and charting code
read it.

`UtcMillis` is a plain `i64` newtype rather than `chrono` because the contract
crate is `no_std` and must not pull `chrono` into the firmware.

Local time exists only in the UI, at render time. Nothing is ever stored in it.

### The `Clock` trait: domain logic never reads the system clock

```rust
pub trait Clock: Send + Sync { fn now(&self) -> DateTime<Utc>; }

pub struct SystemClock;                              // production
pub struct TestClock { now: Arc<Mutex<DateTime<Utc>>> }   // deterministic tests
pub struct AcceleratedClock { anchor: DateTime<Utc>, started: Instant, scale: f64 }
```

This is what makes the safety property tests possible: `safety_006_rolling_24h_cap`
generates a day of command history in microseconds by advancing a `TestClock`,
rather than by sleeping.

Enforcement is not left to discipline — `clippy.toml` disallows
`chrono::Utc::now` and `SystemTime::now` inside `rhizo-domain`
([ADR-001](001-rust-workspace-and-crate-boundaries.md), issue M1-013).

### Accelerated virtual time in the simulator

```text
virtual_now = anchor_real + (real_now - anchor_real) * scale
```

At `scale = 600`, ten simulated minutes pass per real second, so a full
multi-dose watering cycle with two 15-minute absorption waits completes in about
six seconds of wall time. The anchor is a real epoch instant so virtual
timestamps remain plausible UTC values that store and chart normally.

**One clock per process.** Mixing accelerated and system time inside one process
would corrupt the rolling-window computation. In the accelerated test topology,
the edge and the simulator both run accelerated with the same scale, configured
from the same compose variable.

### A device without a synced clock refuses every water command

This is the most consequential decision in this ADR.

```text
if !clock_synced        → reject(clock_unsynced)
if now > expires_at + MAX_CLOCK_SKEW  → reject(expired)
else                    → accept
```

The alternative — treating the TTL as relative to receipt — is unsafe, because
MQTT carries no delivery timestamp. A device cannot distinguish "the broker
delivered this immediately" from "the broker held this for six hours while I was
offline". Since it cannot tell, SAFETY-012 requires it to decline.

Consequences, accepted deliberately:

- A device must complete SNTP sync before it will water. This takes seconds
  after Wi-Fi association and is reported as `clock_synced: true` in status.
- A device that loses SNTP for a long period stops accepting water commands, and
  the edge shows this as a lockout reason. **Monitoring continues normally** —
  telemetry needs no synced clock, because the edge stamps arrival itself.
- The default TTL of 120 s is short enough that a command queued during a
  disconnect is almost always stale on arrival, which is the intent.

`MAX_CLOCK_SKEW_SECONDS = 5` absorbs normal LAN jitter. Divergence between
`device_time_ms` and `received_at` beyond 30 s raises a `clock_skew` event.

### Staleness threshold

```text
max_sample_age = max(15 minutes, 3 × telemetry_interval)
```

Three intervals tolerates two lost messages before locking out — forgiving of
normal packet loss, unforgiving of a dead sensor. The 15-minute floor stops a
device configured with a 10-second interval from locking out on one hiccup.

### Rolling 24-hour window, not calendar day

A calendar-day cap permits two full daily allowances within a few hours around
midnight — 23:50 and 00:10. The window is `now - 24h`, computed by summing
`watering_events.delivered_ml`. Because it is derived from persisted rows rather
than a counter, a restart cannot reset it (SAFETY-006, SAFETY-010).

### Edge clock steps

An NTP step correction is detected by comparing wall-clock movement against
`std::time::Instant` each control tick.

- **Backwards step:** the window includes more history, so the cap becomes more
  conservative. Safe; logged as `clock_step`.
- **Forwards step > 10 minutes:** older watering events fall out of the window
  early, potentially permitting an extra dose. All plants enter `Uncertain`
  lockout for one cooldown period. Uncertainty defaults to not watering.

The asymmetry is deliberate: the safe direction is allowed silently, the unsafe
direction triggers a lockout.

## Alternatives considered

**Trusting device timestamps for staleness.** Rejected — see above; it inverts
SAFETY-005 exactly when it matters.

**Relative TTL (`ttl_ms` from receipt).** Rejected: MQTT provides no delivery
timestamp, so "from receipt" is unknowable in the case that matters.

**Letting an unsynced device water anyway, relying on the edge's check.**
Rejected: it removes the device's independent veto, which is the entire point of
defence in depth. The edge could be the thing that is wrong.

**Storing ISO-8601 TEXT.** Rejected: larger indexes, string comparison, and it
invites accidental naive/local timestamps.

**Storing seconds.** Rejected: insufficient resolution for pump durations and
for ordering messages within a second.

**A logical/Lamport clock instead of wall time.** Rejected: the safety rules are
genuinely about physical elapsed time — soil dries in hours, not in ticks.

**Calendar-day water cap.** Rejected — the midnight double-allowance.

## Consequences

Positive:

- Safety computations depend on one clock the operator controls and can check.
- A wrong device clock degrades to a refusal, never to a permissive decision.
- Domain tests are fully deterministic and fast; no test sleeps for logical time.
- Accelerated time makes a multi-hour scenario a six-second test.

Negative, accepted:

- Devices need working SNTP to water. A LAN without outbound NTP needs a local
  NTP server — documented in M9's deployment notes.
- Two time representations (ms on MQTT, RFC 3339 on HTTP) is a seam where bugs
  can hide. Mitigated by converting in exactly one crate and round-trip testing.
- `Clock` threaded through the domain adds a parameter to many signatures.
  Accepted; it is the price of deterministic safety tests.

## Risks

- **A contributor calls `Utc::now()` in the domain** for convenience, silently
  breaking test determinism. *Mitigation:* the `clippy.toml` disallowed-methods
  guard (M1-013) fails the build.
- **Accelerated and real clocks mixed** in one test topology, corrupting the
  rolling window. *Mitigation:* the scale factor comes from one compose variable
  consumed by both services; M8-004 asserts the edge and simulator report the
  same scale at startup.
- **SNTP unavailable on a home LAN** with restrictive firewall rules, silently
  preventing all watering. *Mitigation:* `clock_unsynced` is a first-class
  lockout reason shown in the UI with a specific remedy, not a generic error.

## Follow-up

- [time-model.md](../architecture/time-model.md) — normative rules.
- M1-003 implements `UtcMillis`; M1-013 adds the clippy guard.
- M3-006 implements `received_at` stamping.
- M6-005 implements staleness; M6-007 the rolling window; M6-015 clock-step detection.
- M8-004 asserts consistent time scale across the test topology.
