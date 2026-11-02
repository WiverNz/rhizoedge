# M9 frozen acceptance inventory — 2026-11-02

M9 **IN PROGRESS**, work paused/deferred at the user's development checkpoint.
M10 **NOT READY**, not started. No acceptance criterion is waived.

This inventory is the current disposition of **every acceptance checkbox** in
M9-001…022 and PRD 090, plus applicable HIL-1/HIL-2 checks. An ID's final suffix
is its one-based order under that file's Acceptance criteria heading. Compound
criteria get the stricter disposition when only part is demonstrated.

- **VERIFIED**: evidence satisfies the stated scope. Host/source verification
  is explicitly distinguished from real ESP32-C3 virtual actuation and GPIO.
- **DEFERRED PHYSICAL/HIL**: required physical observations remain unperformed;
  exact missing evidence is recorded. Deferral is not a safety pass.
- **BLOCKED**: implementation, specification consistency, or non-physical
  verification gap remains. Freezing work does not convert a gap to a pass.
- **NOT APPLICABLE**: conditional scope is not present in this M9 configuration;
  later milestones retain their own requirements.

Historical issue checkmarks were reviewed, not treated as physical proof.
References to host evidence mean the board-free implementation/test record in
[M9 report](M9.md), its checkpoint results, and the linked source issue; they do
not claim exhaustive hardware coverage. The source issue checkboxes reflect
this inventory; detailed evidence/gaps live here to avoid duplicating them.

## Evidence index

- [M9 report](M9.md): board-free gates; October real telemetry, virtual execution,
  durable ledger/retry; November RTC, radio, LWT and sleep work.
- [Current bench](../hardware/m9-current-bench.md): config 18 always-on, pump
  disabled, diagnostic image, unchanged GPIO5 pull-down/probe, all loads absent.
- [PicoScope runbook](../hardware/picoscope-2205a.md): Windows ps2000, known-signal
  scaling, static LOW, failed streaming, limited block capture; no HIL pass.
- Ignored `local/evidence/m9-independent-20261102/sleep-campaign-02/`:
  10 qualifying wakes then harness timeout; original evidence retained.
- Ignored `local/evidence/m9-independent-20261102/wifi-diagnostic-01/`:
  3/3 timer wakes within fixed ADR-018 windows, no reproduced slow cycle,
  diagnostics only, restoration verified. Not a 20-cycle acceptance run.
- Ignored `local/evidence/m9-freeze-20261102/`: checkpoint command logs/results.

## Issue acceptance criteria

### M9-001

[Source issue](../issues/M9/001-verify-esp32-toolchain.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-001.01** Every command in ADR-007 has been executed. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-001.02** ADR-007 is corrected to match reality, including on Windows. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-001.03** A hello-world builds for `riscv32imc-esp-espidf`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-001.04** Exact crate versions are recorded and pinned. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-001.05** The firmware Rust version is resolved: either `1.98.0` or a named ESP-specific channel, pinned in the firmware workspace and recorded in ADR-007's embedded-exception section. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-001.06** The host workspace toolchain is **unchanged** at 1.98.0. | BLOCKED | Historical M9-001 used host 1.98.0; current accepted pin is 1.99.1. Reconcile this stale literal acceptance text with the documented later toolchain migration; no downgrade is requested. |
| **M9-001.07** Build time from cold is measured and noted. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-001.08** The re-verify note in ADR-007's Status is updated with the date. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-002

[Source issue](../issues/M9/002-add-firmware-ci-job.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-002.01** The job builds the firmware for `riscv32imc-esp-espidf`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-002.02** It builds every declared board profile, from the same application code. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-002.03** Adding a board profile is a one-line matrix change. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-002.04** It triggers on the specified paths. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-002.05** Caching keeps warm builds under 5 minutes. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-002.06** A contract change that breaks the firmware fails it. | BLOCKED | Missing the required negative hosted-CI experiment: a deliberate embedded contract incompatibility must fail the firmware job. Positive cached CI is not this proof. |
| **M9-002.07** The Linux-container fallback is documented. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-003

[Source issue](../issues/M9/003-create-firmware-workspace.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-003.01** `cargo build --release` succeeds with no board attached. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.02** The contract crate is a path dependency with default features off. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.03** `cargo test` runs host tests in the firmware workspace. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.04** The protocol fixture corpus runs and passes here too. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.05** `grep -r esp_idf firmware/esp32-node/src/app` returns nothing. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.06** `board-devkitm1` exists, is the documented default profile, and builds. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.07** Building with **no** board feature fails with a `compile_error!` naming the available profiles; building with **two** fails the same way. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.08** Nothing outside `src/board/` names a GPIO number, a pin polarity, or a board-specific peripheral constructor. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.09** The structural board-isolation test exists, fails when a pin literal is planted in `src/app/`, and passes once it is removed. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-003.10** The rest of the firmware obtains peripherals through the board interface and cannot observe which board profile is active. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-004

[Source issue](../issues/M9/004-implement-nvs-and-identity.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-004.01** `device_id` is derived from MAC on first boot and persisted. | BLOCKED | identity::resolve derives from MAC without mutating persisted provisioning; main stores before resolve. Missing derived-identity persistence implementation/regression and first-boot evidence; do not erase existing NVS. |
| **M9-004.02** An NVS override wins over the derived value. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-004.03** `boot_id` changes each boot. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-004.04** Corrupt NVS starts fresh with a log and an event. | BLOCKED | Host boot outcome and production warning exist; production main does not demonstrate enqueue/publication of the nvs_reset audit event. Missing runtime event integration and observable corrupt-store recovery. |
| **M9-004.05** The dedup ring persists across a power cycle. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-004.06** Host tests cover all of this with the fake store. | BLOCKED | Existing fake-store tests pass but do not prove the missing derived-identity write-through and production nvs_reset event. Add regressions when those gaps are addressed. |

### M9-005

[Source issue](../issues/M9/005-implement-hardware-traits-and-fakes.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-005.01** All seven traits are defined. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-005.02** `Clock::now_ms` returns `Option`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-005.03** Fakes exist for every trait and are configurable. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-005.04** Fakes can simulate read errors, stuck values, and no-delivery. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-005.05** Pin assignments live in the board profile, and no trait definition or adapter outside `src/board/` names a pin, port, or polarity. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-005.06** Real adapters are constructed by the board profile and handed to the app as trait objects. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-005.07** Host tests can drive the full app with fakes only, with **no** board profile involved. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-006

[Source issue](../issues/M9/006-add-serial-provisioning.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-006.01** All provisioning commands work and persist to NVS. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-006.02** `show` redacts secrets. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-006.03** Settings take effect after reboot. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-006.04** Provisioning is unavailable at runtime without an explicit unlock. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-006.05** One firmware image works for multiple devices. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-006.06** The procedure is documented. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |

### M9-007

[Source issue](../issues/M9/007-implement-boot-safe-pump-state.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-007.01** `pump_off()` is the first statement in `main`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-007.02** A host test asserts the call ordering. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-007.03** The watchdog is enabled. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-007.04** A watchdog reset path leaves the pump off. | DEFERRED PHYSICAL/HIL | Host watchdog/boot ordering is tested. Missing deliberately triggered watchdog reset with continuous GPIO5 transient evidence; HIL watchdog count is 0. |
| **M9-007.05** The hardware pull-down requirement is documented in the board profile, beside the pump pin it constrains. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-007.06** No initialisation path can energise the pump. | DEFERRED PHYSICAL/HIL | Source ordering and static LOW do not exclude bootloader/reset pulses. Missing complete cold-boot/flash/reset captures under HIL-1. |

### M9-008

[Source issue](../issues/M9/008-implement-wifi-connection.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-008.01** The device connects with NVS credentials. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-008.02** Reconnect uses the documented backoff, with jitter, capped at 300 s. | VERIFIED | Backoff source/host checks: base 2 s, full jitter, cap 300 s; generation 440 observed 1690/3114/1726/3019 ms requests within the respective ceilings. Ten-minute HIL outage is separately deferred. |
| **M9-008.03** Retry is unlimited — the device never stops trying. | VERIFIED | Production outer loop has no retry cutoff; counter saturates. Four failed attempts then recovery observed in generation 440. This is source plus finite recovery evidence, not a claim to physically test infinity. |
| **M9-008.04** Sampling continues while Wi-Fi is down. | VERIFIED | Generation 440 continued isolated sample/evaluation calls during backoff and replayed policy_disabled audit history after recovery. Does not prove a strict sampling cadence during blocking connect calls. |
| **M9-008.05** The pump stays off while disconnected. | DEFERRED PHYSICAL/HIL | Missing production GPIO observation during radio outage under unauthorized actuation conditions. Offline-policy-authorized watering is permitted by ADR-015; the wording must not be read as a blanket ban on that path. |
| **M9-008.06** RSSI is reported in status. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-009

[Source issue](../issues/M9/009-implement-mqtt-client-and-lwt.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-009.01** The device connects with per-device credentials. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.02** The LWT is set **before** connect, asserted by a host test. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.03** `clean_session` is true. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.04** Retained online status is published on connect. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.05** It subscribes to the eight exact topics of protocol §3 and to no wildcard, so no subscription can match a topic it publishes. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.06** It does **not** subscribe to `commands/result`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.07** Killing power produces the LWT within the keepalive window. | BLOCKED | Functional unique LWT/Edge offline evidence exists. Missing trustworthy removal-to-LWT timing with canonical 60 s keepalive, and reconciliation of literal within-keepalive wording with MQTT 1.5x broker timeout. Overnight lwt60 run was incomplete. |
| **M9-009.08** Applying an `edge.time` makes `clock_synced` true. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.09** An `edge.time` older than the last applied one is **ignored**. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.10** An `edge.time` **equal** to the last applied one is ignored and does not refresh `synced_at_monotonic`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.11** `clock_synced` becomes false once the last sync exceeds `TIME_SYNC_MAX_AGE_SECONDS`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-009.12** Withholding `edge.time` leaves `clock_synced` false, reported as such, with telemetry unaffected. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-010

[Source issue](../issues/M9/010-implement-firmware-telemetry.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-010.01** All configured telemetry kinds publish with valid envelopes. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-010.02** `retain` is false on every telemetry publish. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-010.03** `sequence` is monotonic within a boot. | BLOCKED | run::envelope_for uses now_mono as sequence; multiple messages share it. Missing proof/implementation of the protocol-required ordering across all message kinds (not telemetry alone). |
| **M9-010.04** UUIDv7 when synced, v4 with `clock_synced: false` otherwise. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-010.05** At most 16 samples are buffered across a disconnect. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-010.06** Status heartbeat appears on schedule with sensor health. | BLOCKED | Heartbeat exists, but production status capability/health reporting is not demonstrated (default capabilities). Missing sensor-health integration and scheduled heartbeat evidence with real transitions. |
| **M9-010.07** Fixtures generated by the firmware match the corpus. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-011

[Source issue](../issues/M9/011-implement-command-handling-and-dedup.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-011.01** A valid command actuates and reports `completed`. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.02** A duplicate `command_id` republishes the stored result and does **not** actuate. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.03** The ring survives a power cycle. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.04** NVS is written before actuation. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.05** **A failed NVS write aborts the dose.** | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.06** The device daily cap is enforced independently of the edge. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.07** A result is published for every command including rejections. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.08** A published result is retained until `command.result.ack` names it, and is **not** retired by the broker's publish ack. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.09** The pending-result ledger's capacity and saturation behaviour are decided and written down in the M9 report, not left implicit. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.10** **A saturated ledger fails closed**: no unacknowledged watering result is silently discarded in a way that can under-count delivered water. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.11** Whether new actuation is refused while saturated is stated, with its refusal reason, and the tests match the stated choice. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.12** Saturation emits a durable fault or event; already-delivered water remains attributable while it lasts. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.13** The ledger survives a reboot at saturation with no entry dropped or duplicated. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.14** Acknowledgement frees space and restores normal operation cleanly. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.15** If eviction of an unacknowledged result is adopted, its safety equivalence is argued explicitly in the report; otherwise the firmware does not evict. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-011.16** `grep -c validate_water_command` shows exactly one call site. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |

### M9-012

[Source issue](../issues/M9/012-implement-firmware-config-handling.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-012.01** A valid config is applied and persisted. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-012.02** `applied_config_version` is echoed in status. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-012.03** A config with a lower version is ignored. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-012.04** An invalid config is rejected and the previous retained. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-012.05** A config containing `max_ml_per_run` has no effect on the reported limits. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |
| **M9-012.06** Config survives a reboot. | VERIFIED | Existing host tests plus recorded ESP32-C3 provisioning/config or virtual-actuator NVS/ledger evidence in M9 report; no physical pump/GPIO claim. |

### M9-013

[Source issue](../issues/M9/013-implement-interrupted-dose-reporting.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-013.01** An unfinished NVS record on boot produces an `interrupted` result. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-013.02** `delivered_ml` is **null**, not 0. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-013.03** The pump is off before the check runs. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-013.04** The dose is **not** resumed. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-013.05** The record clears after acknowledgement. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-013.06** The result survives a failure to publish and is retried next boot. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-014

[Source issue](../issues/M9/014-add-conformance-test.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-014.01** The scenario runs against both implementations. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-014.02** Published sequences match modulo ids and timestamps. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-014.03** Command reject reasons are identical for every refusal case. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-014.04** A deliberate divergence fails the test. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-014.05** It runs on the host with no board. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-015

[Source issue](../issues/M9/015-implement-nvs-policy-store.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-015.01** A valid policy is staged, verified, activated, and acknowledged. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-015.02** Interruption at every step leaves exactly one valid active policy. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-015.03** A lower or equal `policy_version` is ignored. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-015.04** A corrupt store refuses to activate and substitutes no default. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-015.05** An invalid policy leaves the previous one active. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-015.06** `applied_policy_versions` is reported accurately. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-015.07** All of the above are covered by host tests with no board. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-016

[Source issue](../issues/M9/016-integrate-offline-evaluator.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-016.01** An isolated device with a valid enabled policy waters within bounds. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-016.02** An isolated device with no policy never waters. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-016.03** Elapsed time comes from the monotonic timer, not the wall clock. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-016.04** `grep -c validate_water_command` shows exactly one call site. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-016.05** Every refusal is buffered as an audit event with its reason. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-016.06** Control returns to the edge on reconnection. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-016.07** Host tests cover all of the above with fake adapters. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-017

[Source issue](../issues/M9/017-implement-firmware-event-buffer.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-017.01** Events buffer while isolated and replay on reconnection. | DEFERRED PHYSICAL/HIL | Wire replay of offline.refused event de7c624f-9185-4a60-8f15-f193832fcd02 is recorded; missing complete Edge durable ingestion plus event.ack retirement evidence for the physical buffer. |
| **M9-017.02** `event_id` is identical across repeated replays. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-017.03** Audit events survive a telemetry flood. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-017.04** Overflow emits a gap with the correct range and count. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-017.05** The buffer survives a reboot. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-017.06** Unacknowledged events are replayed again. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-017.07** Audit events are durable across power loss; telemetry may be lost. | DEFERRED PHYSICAL/HIL | Missing controlled complete power loss with buffered unacknowledged audit events, then stable IDs, durable Edge ingestion and event.ack. Command-result persistence is a different ledger. |
| **M9-017.08** This buffer's eviction policy is **not** applied to the pending-result ledger (M9-011); if the two share a storage mechanism, they do not share an overflow decision. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-018

[Source issue](../issues/M9/018-implement-monotonic-budget.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-018.01** A reboot does not replenish `budget_used_ml`. | VERIFIED | Host budget tests plus physical timer, cold USB power loss and invalid-RTC evidence; only those measured reset classes are claimed. |
| **M9-018.02** A reboot does not shorten a cooldown. | VERIFIED | Host budget tests plus physical timer, cold USB power loss and invalid-RTC evidence; only those measured reset classes are claimed. |
| **M9-018.03** The budget replenishes as the window genuinely advances. | VERIFIED | Host budget tests plus physical timer, cold USB power loss and invalid-RTC evidence; only those measured reset classes are claimed. |
| **M9-018.04** A trusted wall clock, once Edge synchronisation is re-established, allows correct window advancement. | DEFERRED PHYSICAL/HIL | Missing physical trusted-resynchronization/window-advancement evidence without granting extra allowance. Existing timer/cold/corrupt-RTC accounting evidence is retained. |
| **M9-018.05** The edge's post-reconciliation baseline is accepted. | VERIFIED | Host budget tests plus physical timer, cold USB power loss and invalid-RTC evidence; only those measured reset classes are claimed. |
| **M9-018.06** Monotonic overflow does not grant budget or clear a cooldown. | VERIFIED | Host budget tests plus physical timer, cold USB power loss and invalid-RTC evidence; only those measured reset classes are claimed. |
| **M9-018.07** Host tests cover repeated reboots at random points. | VERIFIED | Host budget tests plus physical timer, cold USB power loss and invalid-RTC evidence; only those measured reset classes are claimed. |

### M9-019

[Source issue](../issues/M9/019-implement-power-mode-and-deep-sleep.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-019.01** One image serves both power modes, selected by configuration. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-019.02** An absent or unrecognised `power.mode` yields `AlwaysOn`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-019.03** Always-on behaviour is unchanged; M9-001…M9-018 tests stay green. | BLOCKED | Checkpoint host regression is separate evidence; full unchanged always-on behavior cannot close while production rail enable/warmup integration (M9-020) is incomplete. |
| **M9-019.04** The sleep announcement's PUBACK is observed before `esp_deep_sleep`. | VERIFIED | Correct matching packet-ID PUBACK precedes every counted sleep in sleep-campaign-02 and wifi-diagnostic-01; focused adversarial sleep_ack host tests preserve unrelated inbound work. |
| **M9-019.05** A timer wake with a valid checksum credits elapsed time; a cold boot and a corrupted checksum each credit zero. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-019.06** A power cycle mid-cooldown neither shortens the cooldown nor replenishes the budget. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-019.07** `wake_reason` is reported truthfully in status. | DEFERRED PHYSICAL/HIL | Timer and cold-boot reasons observed correctly. Missing remaining required physical reset/wake classes, including watchdog, brownout/external wake as applicable, with conservative accounting verification. |
| **M9-019.08** `deep_sleep` has exactly one call site, checked structurally. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-019.09** `src/app/` still contains no `esp_idf_*` imports. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-019.10** **With a board:** 20 consecutive wake cycles complete with no missed wake and no watchdog reset. | DEFERRED PHYSICAL/HIL | Missing one complete 20-consecutive-wake campaign. Five-cycle run interrupted; next run stopped after ten; three diagnostic wakes passed separately. Do not add these counts. Use ADR-018 fixed Edge-derived deadline (360 s here), not 100 s. |

### M9-020

[Source issue](../issues/M9/020-implement-peripheral-power-gating.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-020.01** Both rails are off during deep sleep, verified by GPIO state before the sleep call. | DEFERRED PHYSICAL/HIL | Missing GPIO6/GPIO7 static levels before sleep and retained sleep-state measurements, plus validated switched-rail fixture. GPIO5 measurement does not prove either rail. |
| **M9-020.02** A read attempted before `sensor_warmup_ms` has elapsed raises `sensor_warmup_incomplete` and does not publish a sample. | BLOCKED | Pure warmup state tests exist; production read_batch path has no demonstrated configured rail/warmup integration with sensor_warmup_incomplete and suppressed early samples. |
| **M9-020.03** An error during sampling still disables both rails. | BLOCKED | Guard error cleanup is host-tested, but physical runtime sampling does not demonstrate owning/enabling rails through that guard. Missing integration plus error-path rail evidence. |
| **M9-020.04** Rails are driven off in the boot-safe sequence. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-020.05** `AlwaysOn` mode leaves rails enabled and its behaviour unchanged. | BLOCKED | Production main disables rails at boot; no demonstrated runtime enable path for always-on sampling. Missing runtime wiring and measured enabled rails. |
| **M9-020.06** No stabilisation constant for any specific sensor part appears in the firmware source. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-020.07** A device with no RS485 capability never enables that rail. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-020.08** Rail pins and polarity come from the board profile; `src/app/` and the sampling code name no GPIO and no active level. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-021

[Source issue](../issues/M9/021-implement-awake-watering-and-battery-telemetry.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-021.01** The device does not sleep while a dose is in progress, including when `awake_budget_seconds` is shorter than the dose. | DEFERRED PHYSICAL/HIL | Host hold/state-machine proof exists. Missing real production benign-actuator active-dose wake test with awake budget shorter than dose and complete sleep/output observation. |
| **M9-021.02** The hold is released on every path, including a refused command, a failed publish, and a panic-free error return. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-021.03** `command.result` is acknowledged before the sleep announcement. | BLOCKED | Hold drops after publish; sleep announcement does not gate on pending durable results. Missing runtime result-ACK/sleep ordering. Clarify broker PUBACK (PRD F-090-57) versus application ACK/durable work, then regress and physically prove canonical behavior. |
| **M9-021.04** `FIRMWARE_MAX_RUN_SECONDS` still ends a run independently of the hold. | DEFERRED PHYSICAL/HIL | Missing benign production actuator run with independent hard timer measured; host timing and virtual actuator are not electrical hard-limit proof. |
| **M9-021.05** A power cut mid-dose yields a boot with the pump off and `status: "interrupted"`, `delivered_ml: null`, reported at the next wake. | DEFERRED PHYSICAL/HIL | Missing mid-dose real power interruption, interrupted/null durable result at wake and boot-OFF transient proof. Completed virtual dose dedup across power loss does not cover this composite criterion. |
| **M9-021.06** `battery_voltage` appears in the telemetry batch on capable hardware and is **absent** on hardware without the divider. | VERIFIED | Conditional current-board branch: no battery divider/capability; voltage absent. Host tests cover the abstraction. Capable-board voltage measurement is not applicable to the current unmodified profile and is not claimed. |
| **M9-021.07** `battery_percent` is absent unless a chemistry curve is configured. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-021.08** `battery_low` is raised once per crossing, not once per sample. | BLOCKED | Missing production battery_low event crossing integration and once-per-crossing regression/observable evidence; no battery hardware fixture is presently configured. |
| **M9-021.09** No battery field appears in `IrrigationInputs` or in any argument to `validate_water_command`, checked structurally. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |

### M9-022

[Source issue](../issues/M9/022-m9-verification.md).

| Criterion | Classification | Evidence / exact missing evidence |
|---|---|---|
| **M9-022.01** `cargo build --release` succeeds for the ESP target with no board. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.02** The CI firmware job passes. | VERIFIED | Hosted CI run 41410668212, firmware job 125162737430 actually built the image; this historical job pass is not a claim of hosted CI for the current uncommitted diagnostics. |
| **M9-022.03** ADR-007's toolchain section has been executed and corrected. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.04** ADR-007's firmware-structure section matches the board layer as built. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.05** `board-devkitm1` builds; zero or two board features fail with a clear `compile_error!` naming the available profiles. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.06** The structural board-isolation check passes: no GPIO number, pin polarity, or board-specific peripheral construction outside `src/board/`. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.07** The `app/` host tests are board-independent — they pass with no board profile selected, and their results do not vary by profile. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.08** CI builds every declared board profile from the same application code. | VERIFIED | Same CI built the sole implemented production profile devkitm1 with profile guards. XIAO is reserved, not a second supported implementation. |
| **M9-022.09** Host tests cover boot safety, interrupted dose, dedup ring, and command validation. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.10** **The pending-result ledger's saturation behaviour is decided, documented, and tested** (M9-011): the ledger fails closed when full, no unacknowledged watering result is silently discarded in a way that can under-count delivered water, saturation is visible as a durable fault, the state survives a reboot at the boundary, and acknowledgement restores normal operation without loss or double-counting. If any eviction of an unacknowledged result was adopted, the report argues its safety equivalence explicitly. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.11** PRD 090 Open question 5 is **resolved**, with the chosen capacity and behaviour recorded there and in ADR-014. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.12** The conformance test shows identical behaviour to the simulator. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.13** **With a board:** it connects, appears online, publishes telemetry, applies config. (Explicit M9 fake-sensor bench image; report §11.) | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.14** **With a board:** a duplicate `command_id` across a power cycle is deduplicated. (Real ESP32-C3 NVS and virtual execution, October 16–17; report §13. No GPIO/pump claim.) | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.15** **With a board:** an oversized command is clamped or rejected. | DEFERRED PHYSICAL/HIL | Virtual rejection/host clamp exists. Missing production GPIO output evidence for an oversized command with validated benign safety inputs and complete interval capture. |
| **M9-022.16** **With a board:** HIL-1 passes with transient capture (DMM alone is insufficient). | DEFERRED PHYSICAL/HIL | HIL-1 remains 0/20 resets, 0 watchdog, 0/10 mid-boot power cuts; cold boot and flashing also need full transient evidence. Streaming is unqualified; block buffer cannot cover full boot at justified resolution. |
| **M9-022.17** **With a board:** blocking the `time` topic (or disconnecting the edge) refuses commands with `clock_unsynced` while telemetry continues. | VERIFIED | Verified at host/source scope by the issue implementation and existing board-free tests; see M9 report. Physical extension, if required, is listed separately below. |
| **M9-022.18** Documentation updated; report recorded. | VERIFIED | Freeze report, criterion inventory and bench state updated; this records incomplete acceptance and does not close M9-022. |

## PRD 090 acceptance criteria

[Source PRD](../prd/090-esp32-rust-firmware.md).

| Criterion | Classification | Evidence / missing evidence |
|---|---|---|
| **PRD090.01** `cargo build --release` succeeds for `riscv32imc-esp-espidf` with no board. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.02** The CI firmware job passes. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.03** The official Espressif ESP32-C3-DEVKITM-1-N4X is the initial development/reference board, and `board-devkitm1` builds. | BLOCKED | Chip/revision/flash and devkitm1 mapping are verified; missing exact official DEVKITM-1-N4X model/provenance confirmation. USB bridge/chip ID alone cannot prove board manufacturer/model. |
| **PRD090.04** All board-specific GPIO/peripheral mapping is isolated behind the board layer. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.05** No file under application, safety, sensor, pump, or networking code contains a concrete GPIO number, and the structural check proves it. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.06** Firmware application logic does not depend on a specific ESP32-C3 development board. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.07** Selecting zero or two board features fails the build with a clear message. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.08** Adding a second ESP32-C3 board requires a new board mapping/profile, not changes to application logic. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.09** Both supported board profiles compile against the same application code once the second board is introduced, and switching profiles changes no MQTT, domain, or safety test result. | NOT APPLICABLE | No second production board profile introduced; XIAO reserved only. Verification becomes mandatory when introduced. |
| **PRD090.10** [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md)'s toolchain section has been **executed** and corrected, including on Windows. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.11** Host tests cover boot safety, interrupted dose, dedup ring, and command validation. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.12** The pending-result ledger's capacity and saturation behaviour are **decided and written down** — in the M9 report and in this PRD's Open question 5 — rather than left implicit in the ring implementation. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.13** A saturated ledger **fails closed**: no unacknowledged watering result is silently discarded in a way that can under-count delivered water, and the decision on whether new actuation is refused while saturated is stated with its refusal reason. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.14** Saturation emits a durable fault or event visible to the edge and to an operator, and already-delivered water stays attributable while it lasts. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.15** The ledger's state at saturation survives a reboot, and a power cycle at the boundary neither drops nor duplicates a result. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.16** Acknowledgement frees space and restores normal operation with no entry lost or double-counted. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.17** If any eviction of an unacknowledged result is adopted, the M9 report **argues its safety equivalence explicitly**; absent that argument, the answer is that the firmware does not evict. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.18** The conformance test shows identical behaviour to the simulator. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.19** **With a board:** it connects, appears online in the edge API, publishes telemetry from fake sensors, applies retained config, and echoes `applied_config_version`. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.20** **With a board:** a duplicate `command_id` across a power cycle is still deduplicated (the NVS ring survived). | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.21** **With a board:** an oversized command is clamped or rejected. | DEFERRED PHYSICAL/HIL | See M9-022.15: production oversized-command GPIO behavior remains unmeasured. |
| **PRD090.22** **With a board:** HIL-1 passes — the pump line never asserts across 20 resets, a watchdog reset, and 10 mid-boot power cuts. | DEFERRED PHYSICAL/HIL | See M9-022.16 and HIL-1: no complete transient campaign. |
| **PRD090.23** **With a board:** withholding `edge.time` causes every water command to be refused while telemetry continues. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.24** Offline policy staging/activation is atomic and a corrupt update never replaces the last valid policy. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.25** The sole firmware `evaluate_offline` call site uses persisted conservative budget/cooldown state and still routes any dose through the shared actuation gate. | VERIFIED | See matching M9 issue rows above and report; board-independent proofs remain scoped to host/source, executed dose proof to real ESP32-C3 virtual actuator. |
| **PRD090.26** Audit events, sealed history gaps, replay progress, and cumulative `event.ack` handling survive reboot with stable `event_id` values. | DEFERRED PHYSICAL/HIL | Host event-ring stability/gap ACK tests exist; missing complete physical event ingestion/ACK plus power-loss audit proof (M9-017). |

## Applicable HIL checks

Formal stage order remains unchanged. Previously authorized independent protocol
observations do not constitute a formal HIL-2 pass before HIL-1. Real soil checks
belong to M10; pump/cup stages HIL-3 onward belong to later hardware stages.

### HIL-1

| Check | Classification | Evidence / missing evidence |
|---|---|---|
| **HIL-1.1** Power on while capturing the pump driver input. **The pump line never goes active during boot**, including the bootloader window before `main` runs. | DEFERRED PHYSICAL/HIL | Full cold power-on/bootloader GPIO5 transient capture missing. |
| **HIL-1.2** Flash while powered. The pump does not pulse during flashing. | DEFERRED PHYSICAL/HIL | Complete flashing/boot transition GPIO5 capture missing. |
| **HIL-1.3** Press reset repeatedly, 20 times. The pump never actuates. | DEFERRED PHYSICAL/HIL | Literal required count: 0/20 measured resets. |
| **HIL-1.4** Trigger a watchdog reset (a deliberate hang). The pump line goes inactive and stays inactive. | DEFERRED PHYSICAL/HIL | Literal required count: 0 watchdog reset; deliberate hang/reset GPIO capture missing. |
| **HIL-1.5** Pull power mid-boot, 10 times. No actuation on any restart. | DEFERRED PHYSICAL/HIL | Literal required count: 0/10 measured mid-boot power cuts. |

### HIL-2

| Check | Classification | Evidence / missing evidence |
|---|---|---|
| **HIL-2.1** Device connects to Wi-Fi and appears online in the edge API. | VERIFIED | Real ESP32-C3 Wi-Fi/MQTT online/API evidence. |
| **HIL-2.2** Edge time sync applies; status reports `clock_synced: true`. Record how long this takes after connect. | VERIFIED | Physical fresh edge.time and synced status; connection/sync timing in retained November logs. |
| **HIL-2.3** Soil readings appear and are physically plausible: probe in dry air reads low, in a glass of water reads high, in damp soil reads between. | NOT APPLICABLE | Real soil probe integration is explicitly outside M9-022; required in M10, not waived. |
| **HIL-2.4** Retained status survives an edge restart. | VERIFIED | November retained online status survived Edge restart with same board boot. |
| **HIL-2.5** LWT fires within the keepalive window when power is cut. | BLOCKED | Functional unique LWT/Edge offline evidence exists. Missing trustworthy removal-to-LWT timing with canonical 60 s keepalive, and reconciliation of literal within-keepalive wording with MQTT 1.5x broker timeout. Overnight lwt60 run was incomplete. |
| **HIL-2.6** Retained config is received and `applied_config_version` echoes it. | VERIFIED | Physical retained configs and echoed versions through config 18. |
| **HIL-2.7** Disconnect Wi-Fi for 10 minutes. The device reconnects with backoff and does **not** flush a large telemetry backlog. | DEFERRED PHYSICAL/HIL | Missing literal ten-minute Wi-Fi outage/reconnect and bounded-backlog observation. |
| **HIL-2.8** Withhold `edge.time` (stop the edge, or block the `time` topic). Confirm `clock_synced: false` is reported after the max age and telemetry continues. | DEFERRED PHYSICAL/HIL | Startup without time was proved; missing physical elapsed TIME_SYNC_MAX_AGE_SECONDS=1800 expiry while telemetry continues. |

## Additional requirement gaps that checklist aggregation must not hide

| Requirement | Classification | Exact remaining work |
|---|---|---|
| M9-019 scope: learned BSSID/channel RTC fast reconnect with scan fallback | BLOCKED | No retained AP hint in production Station/RTC state. Implement/verify learned, validity-checked hints and fallback without hard-coded AP/security changes, if retained as canonical scope. Not demonstrated cause of the delayed wake. |
| F-090-60: NVS on change/watering, not every wake | BLOCKED | main persists boot state and announce_and_sleep persists state per cycle. Reconcile boot-generation/durability and flash-wear requirement, implement the canonical policy and regress; current timer tests do not prove write endurance. |
| Physical executed GPIO duplicate and hard-limit interval | DEFERRED PHYSICAL/HIL | Virtual-actuator dose dedup is complete, but production safe-input/benign electrical fixture and complete output capture are still missing. No water-moving load authorized. |
| Remaining RTC reset domains | DEFERRED PHYSICAL/HIL | Watchdog/brownout/external-wake behavior not physically evidenced; retain conservative budget/cooldown and reject invalid RTC in each required class. |
| Capable-board battery-voltage branch | NOT APPLICABLE | Current board profile has no divider. If added, real voltage scaling and capability-path testing become required; absence is the only current-board claim. |

M9 cannot be DONE while any applicable criterion remains deferred or blocked.
All counts above are criterion counts, not tests; overlap between issues, PRD
and HIL is intentional. No physical test was repeated to produce this inventory.

Inventory totals (issue + PRD + HIL rows only): BLOCKED: 16, DEFERRED PHYSICAL/HIL: 24, NOT APPLICABLE: 2, VERIFIED: 179.
