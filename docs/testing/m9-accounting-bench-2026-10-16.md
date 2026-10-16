# M9 RTC accounting bench — 2026-10-15–16

Selected serial evidence; bare board, pump disabled. This is not HIL-1.

## after-power

```text
I (614) esp32_node: wake_reason ColdBoot, credited 0ms; accounting used_ml=40 window_ms=0 cooldown_ms=120000 rtc_valid=false
```

## timer

```text
I (614) esp32_node: wake_reason ColdBoot, credited 0ms; accounting used_ml=40 window_ms=0 cooldown_ms=120000 rtc_valid=false
I (613) esp32_node: wake_reason Timer, credited 60603ms; accounting used_ml=40 window_ms=60603 cooldown_ms=59397 rtc_valid=true
I (613) esp32_node: wake_reason Timer, credited 60603ms; accounting used_ml=0 window_ms=31206 cooldown_ms=0 rtc_valid=true
```

## invalid

```text
I (614) esp32_node: wake_reason ColdBoot, credited 0ms; accounting used_ml=40 window_ms=0 cooldown_ms=120000 rtc_valid=false
W (26034) esp32_node::hal::sleep: BENCH: deliberately corrupting retained checksum
I (614) esp32_node: wake_reason Timer, credited 0ms; accounting used_ml=40 window_ms=0 cooldown_ms=120000 rtc_valid=false
W (25964) esp32_node::hal::sleep: BENCH: deliberately corrupting retained checksum
```
