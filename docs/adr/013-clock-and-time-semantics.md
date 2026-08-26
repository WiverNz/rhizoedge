# ADR-013 — Clock and time semantics

## Status

Accepted — 2026-08-25. Implemented across M1 (representation), M3 (stamping),
M6 (staleness, TTL, rolling window).

**Extended 2026-08-26** with two additions required by device offline autonomy
([ADR-015](015-device-offline-autonomy.md)): devices obtain wall time **from the
Edge over the existing MQTT connection**, so a site-offline outage does not
disable watering; and **offline autonomy runs on monotonic elapsed time** rather
than a wall clock. Everything below about edge-clock authority, staleness, and
the rolling window is unchanged.

**Superseded within the same day:** the first version of this extension had the
Edge run an **NTP daemon** for the LAN. That was reopened before M1 and replaced
with time synchronisation over MQTT — same guarantee, materially less to build
and operate. See §The Edge is the site's time source, and
[mqtt-v1.md](../protocol/mqtt-v1.md) §5.12 for the wire mechanism.

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

### The Edge is the site's time source — over MQTT, not NTP

**Devices get their wall clock from the Edge, on the MQTT connection they already
have.** No NTP client on the device, no NTP daemon on the Edge, no public pool,
and no time-server configuration field.

The problem being solved: if devices synced from the internet, then losing the
internet — mode B in
[connectivity-modes.md](../architecture/connectivity-modes.md) — would leave
every device with an unsynced clock and SAFETY-002 would refuse every water
command site-wide. **An internet outage would become an irrigation outage**,
which is exactly the coupling this project exists to prevent.

The Edge is already the authority for staleness, for the rolling window, and for
`received_at`. Making it the authority for the device's wall clock is consistent
rather than novel.

**Why MQTT rather than NTP.** The requirement is only that a device's clock be
good enough to evaluate an absolute `expires_at` against a 5-second skew
allowance on a command with a 120-second TTL. That is an enormously loose target:

```text
one-way MQTT latency on a LAN     ~ milliseconds
oscillator drift over 30 minutes  ~ 180 ms at a poor ±100 ppm
MAX_CLOCK_SKEW_SECONDS            5 s
```

A single timestamp delivered over the existing connection clears that by three
orders of magnitude. Running a real NTP server would buy accuracy the system has
no use for, at the cost of a daemon to install, supervise, and firewall on a
Raspberry Pi, plus an SNTP client and its failure modes in firmware.

**Mechanism** (normative detail in [mqtt-v1.md](../protocol/mqtt-v1.md) §5.12):

```text
device connects → publishes retained device.status
Edge sees the status → publishes edge.time to that device   (live, retain=false)
device applies it, records the monotonic instant
Edge repeats every TIME_SYNC_INTERVAL_SECONDS while the device is online
```

Four properties do the safety work:

1. **Never retained.** A retained timestamp is stale the moment it is stored, and
   a reconnecting device applying one would set its clock backwards to the
   publication time — making expired commands appear valid.
2. **Strictly increasing.** An `edge.time` whose `edge_time_ms` is **less than
   or equal to** the last applied one is ignored entirely — the clock is not set
   *and* `synced_at_monotonic` is not refreshed. The strictness is the point.
   QoS 1 permits redelivery, so the same value can arrive repeatedly; if an equal
   value extended the validity window, one captured message replayed indefinitely
   would hold `clock_synced` true forever while the device learned nothing new
   about the Edge's clock. The window must measure synchronisation freshness, not
   message arrival. The `<` half of the rule is the ordering defence: MQTT does
   not guarantee ordering across a reconnect, so a delayed message can arrive
   after a newer one, and refusing to move the clock backwards fails safe —
   a device clock slightly *ahead* expires commands sooner. Two genuinely
   distinct synchronisations are 300 s apart, so equality never occurs in normal
   operation and the strict rule costs nothing.
3. **Age-bounded validity.** `clock_synced` means the last applied
   synchronisation is younger than `TIME_SYNC_MAX_AGE_SECONDS`, measured on the
   monotonic clock. It no longer means "an SNTP transaction succeeded".
4. **No request topic.** The device's existing retained status is the trigger. A
   device lacking synchronisation republishes its status, rate-limited, rather
   than introducing a second way to ask the same question.

Constants live in `rhizo-mqtt-contract` and are **not configurable**:
`TIME_SYNC_INTERVAL_SECONDS = 300`, `TIME_SYNC_MAX_AGE_SECONDS = 1800`. The max
age is not bounded by drift — it bounds how long a device may keep accepting
commands without confirming the Edge is still there and still agrees.

### Offline autonomy uses monotonic time, not wall time

An isolated device (mode C) cannot refresh its clock. Naively that would disable
autonomous watering for the same reason it disables commands — making the whole
offline-autonomy capability useless in exactly the scenario it exists for.

The resolution is that **every offline rule is a duration**:

| Rule | Clock needed |
|---|---|
| dry confirmation | monotonic |
| hysteresis dwell | monotonic |
| cooldown between cycles | monotonic |
| absorption wait | monotonic |
| measurement staleness | monotonic |
| rolling volume window | monotonic + persisted accumulator |
| **edge command TTL** | **wall clock — unchanged; still refused when unsynced** |

A monotonic timer measures durations correctly without knowing the date. So an
isolated device with an unsynced wall clock **may** act autonomously while still
refusing every edge command it cannot TTL-validate. SAFETY-002 is untouched.

**Across a reboot** the monotonic clock resets. The device therefore persists the
budget accumulator and the cooldown as a *remaining duration*, and on boot
without a trustworthy wall clock **assumes no time has passed**: the cooldown
resumes from its stored remainder and the budget is not replenished. A reboot can
only ever delay watering, never grant more of it — the conservative direction
(SAFETY-015).

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

- A device must be synchronised to the Edge before it will water. That normally
  happens within a second of connecting, because the Edge sends `edge.time` as
  soon as it sees the device's status, and it is reported as `clock_synced: true`.
- A device whose synchronisation ages out past `TIME_SYNC_MAX_AGE_SECONDS` stops
  accepting water commands, and the edge shows this as a lockout reason.
  **Monitoring continues normally** — telemetry needs no synced clock, because
  the edge stamps arrival itself.
- Losing MQTT stops the refresh, so an isolated device eventually reports
  `clock_synced: false`. That is correct and harmless: no Edge command can reach
  it anyway, and offline autonomy runs on monotonic time. On reconnect, commands
  stay refused until a fresh `edge.time` is applied.
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

**An Edge-hosted NTP daemon.** Considered and adopted briefly on 2026-08-26, then
rejected before M1: it meets the requirement, but adds a service to install,
supervise, and firewall on the edge host, plus an SNTP client in firmware, in
exchange for microsecond accuracy against a 5-second allowance. The MQTT
mechanism uses a connection that must already exist for anything to work.

**Full NTP algorithm over MQTT** — round-trip estimation, offset filtering,
clock discipline. Rejected as unjustified: the error budget is three orders of
magnitude wider than the mechanism's worst case, and a clock algorithm in
firmware is a maintenance liability with no benefit here.

**A dedicated device→edge time-request topic.** Rejected: the retained
`device.status` a device already publishes on connect is a clean trigger, and a
second topic would be a second way to express the same intent.

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

- A device only accepts Edge commands while it is in contact with the Edge — the
  same connection carries both, so there is no case where commands arrive but
  time does not. There is no separate time service to deploy, firewall, or
  debug, and no dependency on outbound NTP from the site.
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
- **A stale or replayed `edge.time`** moving a device clock backwards, making an
  expired command appear valid, **or a duplicate keeping `clock_synced` alive
  without a genuinely newer Edge timestamp.** *Mitigation:* the strictly-increasing
  acceptance rule — an `edge_time_ms` less than *or equal to* the last applied one
  is ignored and does not refresh `synced_at_monotonic` — plus two tests: one
  replaying an older timestamp, one replaying the *same* timestamp indefinitely and
  asserting `clock_synced` still ages out (SAFETY-002).
- **A device silently drifting out of synchronisation** because refreshes are
  lost, then refusing every command. *Mitigation:* `clock_synced` is reported in
  every status heartbeat and surfaced as a first-class lockout reason in the UI;
  the device also republishes its status while unsynchronised, which re-triggers
  the Edge push.
- **An accidental `retain: true` on the `time` topic**, which would be the single
  most damaging mistake available in this mechanism. *Mitigation:* stated twice
  in the protocol, asserted by the existing retained-topic integration test
  (M2-010) extended to cover `time`.

## Follow-up

- [time-model.md](../architecture/time-model.md) — normative rules.
- M1-003 implements `UtcMillis`; M1-013 adds the clippy guard.
- M3-006 implements `received_at` stamping.
- M6-005 implements staleness; M6-007 the rolling window; M6-015 clock-step detection.
- M8-004 asserts consistent time scale across the test topology.
