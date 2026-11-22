# Dependency Graph

Execution policy revised 2026-11-22: use required interfaces and the current
prototype scope, not an all-milestones-complete chain. [ROADMAP](../../ROADMAP.md)
is the current priority and acceptance plan. Each issue's Dependencies section
lists implementation prerequisites; a partly verified M9 issue is usable where
its relevant interface is already demonstrated. This never establishes physical
pump safety.

```text
M0…M8 delivered + existing M9 interfaces
  ├─ M10-001 → 002 → 003 → 004 → 006 → 007 → 008/009 → 013
  │    real SEN0601 readings → local MQTT/SQLite/API smoke
  ├─ M11-001…008 (driver, run guard, calibration, tank/leak inputs)
  └─ M12-001/002 → minimal plant view/configuration/water/stop
             │
real readings + bounded driver + essential cup checks (M11-009…012)
             ↓
M11-013/014: supervised plant experiment with minimal M12 interface
             ↓
actual usage → incremental recovery/reliability improvements
```

Issue numbers are stable identifiers, not execution gates;
parallel branches need only their listed prerequisites. M10-005 and
M10-010…012 are deferred and do not gate M10-013. M11 adapter work does not wait
for M10-013, although closed-loop water requires real sensor readings.
M12 does not wait for M11; its minimal exit does not wait for installers or
charts. M13…M16 and extended M9 closure are deferred as listed in ROADMAP.

## Immediate issue order

1. M10-001 uses existing M9-005 traits, M9-010 telemetry and M9-012 config.
2. M10-002/003/004 add Modbus framing, SEN0601 mapping and the real adapter.
3. M10-006/007/008/009 provide usable, configured and fault-aware measurements;
   M10-013 demonstrates the local sensor slice.
4. M11-001/002/003/004 implement DFR0523 actuation, guard, faults and calibration;
   M11-005/006/007/008 integrate required physical tank/leak inputs.
5. M12-001/002/003/004/006/008/010/013/014/019 deliver the minimal interface.
6. M11-009/010/011/012 check startup/control safety and bounded cup watering;
   M11-013/014 complete a supervised real-plant cycle.

Historical M9 completion criteria remain in [M9-022](../issues/M9/022-m9-verification.md)
and its [acceptance inventory](../reports/M9-acceptance-freeze.md). M9-022 is no
longer a dependency of M10-001. No unfinished criterion was converted to passed.
