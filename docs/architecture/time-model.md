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
| **Device wall clock** | SNTP over Wi-Fi, may be unset | Only when `clock_synced == true` | evaluating command TTL on-device; advisory telemetry timestamp |
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

## 4. Command TTL

```text
edge issues:   issued_at = edge_now
               expires_at = issued_at + profile.command_ttl (default 120 s)

device checks: if !clock_synced         → reject(clock_unsynced)   ◄ SAFETY-012
               if device_now > expires_at → reject(expired)        ◄ SAFETY-002
               else                     → accept
```

### Why a device without a synced clock refuses

The alternative — accepting a TTL as a relative duration from receipt — is not
safe, because the device cannot distinguish "the broker delivered this
immediately" from "the broker held this for six hours while I was offline". MQTT
gives no delivery timestamp. Since the device cannot tell, it must decline.

Consequences, accepted deliberately:

- The device must complete SNTP sync before it will water. This is a few seconds
  after Wi-Fi association and is reported in the status message as
  `clock_synced: true`.
- A device that loses SNTP for a long period stops accepting water commands, and
  the edge surfaces this as a lockout reason. Monitoring continues normally —
  telemetry does not require a synced clock.
- The default TTL of 120 s is short enough that a queued command is almost
  always stale by the time a reconnecting device sees it, which is the intent.

### Clock skew tolerance

Devices and edge are both NTP/SNTP-synced on the same LAN, so skew is
milliseconds in practice. The device tolerates `expires_at` up to
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
