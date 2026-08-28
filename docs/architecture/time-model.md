# Time Model

Time semantics are safety-relevant. Command expiry (SAFETY-002), sample
staleness (SAFETY-005), and the rolling daily cap (SAFETY-006) are all
time-derived, so ambiguity here becomes a watering bug.

Decision record: [ADR-013](../adr/013-clock-and-time-semantics.md).

---

## 1. Three clocks, one authority

| Clock | Source | Trustworthy? | Used for |
|---|---|---|---|
| **Edge clock** | host system time, NTP-synced | Yes — authoritative | staleness, daily cap, command issue/expiry, all storage |
| **Device wall clock** | **synchronised from the Edge over MQTT**, may be unset or aged out | Only when `clock_synced == true` | evaluating command TTL on-device; advisory telemetry timestamp |
| **Device monotonic** | uptime since boot | Yes, but relative | ordering within a boot session, pump run duration |

**Rule: the edge stamps `received_at` on every message from its own clock, and
every safety computation uses `received_at`.** A device timestamp is never used
to decide whether data is fresh — a device with a wrong clock could otherwise
make stale data look current, defeating SAFETY-005.

The device timestamp is still stored (as `device_time_ms`) because it is useful
for diagnosing clock drift and for reconstructing device-side ordering.

---

## 2. Representation

- **Storage:** `INTEGER` — Unix epoch **milliseconds, UTC**. Chosen over
  ISO-8601 TEXT for index efficiency and unambiguous comparison, and over
  seconds because pump durations and message ordering need sub-second
  resolution.
- **In `rhizo-mqtt-contract`:** `UtcMillis(i64)` — a plain integer, because the
  crate is `no_std` and must not pull in `chrono`.
- **In `rhizo-domain` and host code:** `chrono::DateTime<Utc>`.
- **On the wire (JSON):** integer milliseconds, e.g. `"device_time_ms": 1756121400000`.
- **In the REST API:** RFC 3339 strings with `Z`, e.g. `"2026-08-25T11:30:00Z`",
  because APIs are read by humans and by charting code.

Never store local time. Never store a naive timestamp. Timezone is a
presentation concern handled in the UI only.

---

## 3. The `Clock` trait

All domain logic takes time through this trait, never from `Utc::now()`:

```rust
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;              // production
pub struct TestClock { … }           // manual set/advance, deterministic tests
pub struct AcceleratedClock { … }    // simulator: real epoch + scale factor
```

`AcceleratedClock` is defined as:

```text
virtual_now = anchor_real + (real_now - anchor_real) * scale
```

With `scale = 600`, ten minutes of simulated drying take one second of wall
time. The anchor is a real epoch instant so that virtual timestamps remain
plausible UTC values and can be stored and charted normally.

**Constraint:** the `Clock` implementation is chosen once at process start and
shared. Mixing accelerated and system time within one process would make the
daily cap computation nonsense.

---

## 3b. Where the device's wall clock comes from

Not from NTP. The Edge publishes `edge.time` on the live, **never retained**
`time` topic — triggered by the device's own retained status and repeated every
`TIME_SYNC_INTERVAL_SECONDS` while it is online
([mqtt-v1.md](../protocol/mqtt-v1.md) §5.12,
[ADR-013](../adr/013-clock-and-time-semantics.md)).

```text
device connects → publishes retained device.status
Edge sees it    → publishes edge.time (retain=false, QoS 1)
device applies  → records the monotonic instant → clock_synced = true
Edge repeats every 300 s while the device is online
```

Three rules make it safe:

- **Never retained.** A retained timestamp would set a reconnecting device's
  clock backwards to the publication instant, making expired commands look valid.
- **Strictly increasing.** An `edge.time` whose `edge_time_ms` is less than **or
  equal to** the last applied one is ignored: the clock is not set and
  `synced_at_monotonic` is not refreshed. `<` stops a delayed message rolling the
  clock back — a device clock slightly *ahead* of the Edge expires commands
  sooner, the safe direction. `==` stops a QoS 1 duplicate, replayed
  indefinitely, holding `clock_synced` true without the device ever learning
  anything new about the Edge's clock. Only a strictly newer Edge timestamp
  extends the validity window.
- **Age-bounded.** `clock_synced` is false once the last applied synchronisation
  is older than `TIME_SYNC_MAX_AGE_SECONDS` (1800 s), measured on the monotonic
  clock.

No round-trip estimation or NTP-style discipline: the error budget here is three
orders of magnitude wider than the mechanism's worst case, and a clock algorithm
in firmware would be complexity with no benefit.

---

## 4. Command TTL

```text
edge issues:   issued_at = edge_now
               expires_at = issued_at + profile.command_ttl (default 120 s)

device checks: if !clock_synced         → reject(clock_unsynced)   ◄ SAFETY-012
               (clock_synced == last edge.time applied < 1800 s ago)
               if device_now > expires_at → reject(expired)        ◄ SAFETY-002
               else                     → accept
```

### Why a device without a synced clock refuses

The alternative — accepting a TTL as a relative duration from receipt — is not
safe, because the device cannot distinguish "the broker delivered this
immediately" from "the broker held this for six hours while I was offline". MQTT
gives no delivery timestamp. Since the device cannot tell, it must decline.

Consequences, accepted deliberately:

- The device must be synchronised to the Edge before it will water. That normally
  happens within a second of connecting — the Edge sends `edge.time` as soon as it
  sees the device's retained status — and is reported as `clock_synced: true`.
- A device whose synchronisation ages out past `TIME_SYNC_MAX_AGE_SECONDS` stops
  accepting water commands, and the edge surfaces this as a lockout reason.
  Monitoring continues normally — telemetry does not require a synced clock,
  because the edge stamps arrival itself.
- The default TTL of 120 s is short enough that a queued command is almost
  always stale by the time a reconnecting device sees it, which is the intent.

### A sleeping device does not change any of this

A battery device is reachable for a few seconds out of every wake interval
([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)). The obvious worry
is that a 15-minute interval makes a 120-second TTL useless.

It does not, because **the command is minted when the device is awake, not when
the operator asks.** The Edge holds the request as a durable *intent* and, at the
next wake, re-runs the safety gate and issues a command whose `issued_at` is the
wake instant. The device receives a command a few seconds old from an Edge it has
just synchronised with, and evaluates it under exactly the rules above.

```text
operator request ──── minutes ────► wake ──► edge.time ──► command (fresh TTL)
        │                                                       │
   intent_expires_at                                       expires_at
   edge clock, operator-visible,                     wire TTL, device-validated,
   never on the wire                                 unchanged at 120 s
```

Two expiries, deliberately not merged into one field. `intent_expires_at`
(default `2 × wake_interval_seconds`, floor 30 minutes) bounds how long the Edge
will keep trying; `expires_at` is the unchanged wire TTL that SAFETY-002 rests
on. `intent_expires_at` never reaches a device.

Consequences worth stating:

- **No change to TTL, to `edge.time`, or to SAFETY-002 was required.** The
  latency lives in an Edge-side record, not on the wire.
- The wake ordering is `edge.time` first, then the command. A `clock_unsynced`
  refusal inside one awake window is a retryable delivery failure, not a terminal
  one — the device is awake and has now been synchronised.
- With a wake interval below `TIME_SYNC_MAX_AGE_SECONDS` (1800 s) a battery
  device stays continuously synchronised. Above it, `clock_synced` ages out
  *between* wakes, which is harmless: nothing can reach the device then, and it
  re-synchronises on connect before any command is delivered.
- The retained sleep announcement carries `expected_wake_ms` as a **diagnostic**.
  It is not a time source. No field of any retained message may set a clock —
  only `edge.time`, which is never retained, does that (§3b).

### Clock skew tolerance

Devices take their wall clock directly from the edge over MQTT, so skew is
one-way broker latency — milliseconds on a LAN — plus oscillator drift since the
last refresh, about 180 ms over the full 1800 s max age even at a poor ±100 ppm.
Both are three orders of magnitude inside the allowance. The device tolerates `expires_at` up to
`MAX_CLOCK_SKEW = 5 s` in the past before rejecting, to absorb jitter. Skew
larger than 30 s between `device_time_ms` and `received_at` raises a
`clock_skew` device event.

---

## 5. Staleness

```text
sample_age = edge_now - measurement.received_at

max_sample_age = max(15 min, 3 × device.telemetry_interval)
```

Three intervals allows two lost messages before lockout — tolerant of normal
packet loss, intolerant of a dead sensor. The 15-minute floor prevents a device
configured with a 10-second interval from locking out on a single hiccup.

`sample_age` is computed from `received_at`, never `device_time_ms`
(see §1).

---

## 6. The rolling 24-hour window

```sql
SELECT COALESCE(SUM(delivered_ml), 0)
FROM watering_events
WHERE plant_id = ?
  AND completed_at > ?   -- edge_now_ms - 86_400_000
  AND mode IN ('automatic', 'recommended');
```

Rolling rather than calendar-day: a midnight boundary would permit two full
daily allowances a few hours apart. See SAFETY-006.

Manual watering is recorded but excluded from the automatic cap, since a human
explicitly chose it; it is still bounded by the device's own
`FIRMWARE_MAX_DAILY_ML` (SAFETY-007), which counts everything.

---

## 7. Behaviour when the edge clock jumps

An NTP step correction can move the edge clock backwards or forwards.

- **Backwards jump:** the rolling-window query naturally includes more history,
  so the cap becomes *more* conservative. Safe; log a `clock_step` event.
- **Forwards jump:** older watering events fall out of the window early,
  potentially permitting an extra dose. Mitigation: the edge records
  `clock_step` events and, on a detected forward step larger than
  `CLOCK_STEP_LOCKOUT_THRESHOLD` (default 10 minutes), places all plants in
  `Uncertain` lockout for one cooldown period. Uncertainty defaults to not
  watering (SAFETY-012).

Detection: a monotonic-clock reference (`std::time::Instant`) is sampled next to
the wall clock each control tick; a divergence beyond threshold is a step.

---

## 8. Determinism in tests

- Unit and property tests use `TestClock` exclusively. No test sleeps to advance
  logical time.
- Integration tests use `AcceleratedClock` with a documented scale.
- A test that requires real elapsed wall time must justify it in a comment; the
  default review answer is "advance the `TestClock` instead".
- The simulator exposes its scale factor over its control interface so scenario
  tests can assert on virtual time.

Anti-goal: `tokio::time::sleep(Duration::from_secs(3600))` in a test suite.
