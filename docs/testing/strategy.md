# Testing Strategy

> Preserve the current manual/automatic distinction: invalid soil readings
> block automatic watering. A supervised manual request still passes the existing
> shared validator and all required tank/leak, duration, budget and cooldown checks;
> it is not a force/bypass path. Do not silently change the protocol safety contract.


Permanent policy, 2026-11-22: testing supports delivery of working features and
essential safety. It is proportional to changed behavior and actual risk.
[ROADMAP](../../ROADMAP.md) defines prototype scope and deferred hardening.
This supersedes blanket local full-suite and exhaustive milestone gates.

## Ordinary changes

Run relevant unit tests and a basic integration smoke for the changed path.
Add tests for important behavior, regressions and realistic failures. Reuse
existing evidence when behavior, configuration and wiring are unchanged.
Repeat or broaden checks only for a relevant change, failure or concrete
unresolved concern. No large matrices or extensive reports for minor changes.

Use real Mosquitto and SQLite for integration claims. Set
`RHIZO_REQUIRE_BROKER=1` for selected broker tests; skipped infrastructure is
not a pass. Preserve deterministic `TestClock` use, shared validators, existing
property tests and regression cases. Run the relevant safety tests when their
behavior changes, without automatically rerunning all scenarios or mutations.

Firmware application tests live in `firmware/node-app`, separately from the
root workspace and the ESP build. Build the ESP image when firmware integration
changes require it. Reuse proven commands from [firmware instructions](../../firmware/README.md).
Do not substitute an unexecuted example command for actual verification.

For documentation-only changes run `cargo run -p rhizo-docscheck`, review the
diff, and check LF line endings. Product builds, Docker and hardware tests are
unnecessary. Existing CI workflows remain configured as-is; consult their path
filters rather than treating every CI job as a required local rerun.

## Essential hardware checks

Before connection/power, follow the local inventory and hardware status rules.
Verify actual pin markings, connector pinouts, polarity, measured supply voltage,
logic compatibility, rated wiring, grounding and isolation. Do not infer wiring
from inventory or firmware alone. Keep the pump disabled while bringing up sensors.

Before enabling pump power, check the actual driver's startup/reset/watchdog and
loss-of-control-signal behavior with the motor disabled. A GPIO static LOW or a
pull-down alone is not proof of a DFR0523 stop. Resolve ambiguous pulse timing
before outputting control signals. Provide a reachable power disconnect.

Start with water into a measuring cup, then small quantities on one plant under
human supervision. Demonstrate bounded runtime independent of MQTT, per-dose and
cumulative volume limits, cooldown/frequency, manual stop, required input faults,
communication loss and conservative restart without replaying a dose. Check
required tank/leak inputs on the real system; absent/unknown inputs remain a
refusal. Calibration provides estimated volume; do not call it verified delivery.

The detailed [HIL catalogue](hardware-in-the-loop.md) remains available for
specific checks. Its extended repetition counts, endurance, exhaustive fault
injection, temperature matrices and power profiling are future hardening unless
a concrete immediate safety risk requires one. Sensor integration does not wait
for pump HIL; pump actuation still waits for essential electrical/control checks.

## Evidence and progression

Keep a short result in the current issue: configuration tested, observed result,
checks run and remaining limits. Link existing evidence instead of generating
new extensive reports. Mark only demonstrated criteria complete; leave deferred
criteria unchecked. Keep historical ADRs and reports intact.

**Prototype tested** means the stated hardware/configuration worked in supervised
use with essential protections. **Production validated** requires broader,
separately scoped evidence and is not the prototype goal. No unattended autonomous
watering until essential fail-safe behavior is demonstrated. Use real experience
to choose the next reliability improvement.

Long-duration HIL, mutation campaigns, exhaustive conformance, endurance and
power optimization remain available in their existing documents/issues. They
are not universal feature-entry or milestone-exit requirements.
