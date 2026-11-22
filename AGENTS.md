# Rhizo Edge — Development instructions

Build a usable real-plant prototype first. This permanent strategy, adopted
2026-11-22, supersedes older all-criteria milestone gates and M9-only session
restrictions. [ROADMAP.md](ROADMAP.md) owns priority and scoped acceptance.
M0–M8 are delivered, M9 remains incomplete, and M10-001 is the next coding task.
M10/M11/M12 may proceed on available interfaces without full historical M9 closure.

## Focus and autonomy

- Implement first; investigate only what the current feature needs. Prefer
  small focused changes and the simplest working solution.
- Read ROADMAP's current path, the current issue and relevant code. Consult
  PRDs, ADRs and protocol sections when their behavior is touched. Do not repeat
  repository-wide audits, history reviews or full builds at session start.
- Reuse existing decisions, infrastructure, implementations and evidence.
  Avoid unnecessary web research, speculative failure scenarios, new milestones,
  new validation issues and expansive reports.
- Resolve routine engineering choices autonomously within the requested scope.
  Ask only for information or authority genuinely needed. Finish the requested
  feature; do not expand scope or start unrelated milestones.
- Summarize briefly: what changed, what was tested, what remains. Never claim a
  test or physical observation that did not happen.

## Source of truth

ROADMAP and the current PRD/issue prototype scope supersede old sequencing and
validation requirements. Normative protocol, implemented safety behavior and
accepted architecture decisions still govern implementation. Historical reports,
freeze inventories and prompts are evidence/provenance, not current entry gates.
Keep ADRs and reports intact; document supersession in current planning text.
Read only the component and failure/time/offline contracts relevant to the change.

## Architecture constraints

- Production/runtime implementation is Rust. Do not add Go, Node.js, TypeScript, npm, or `package.json` without an explicit architecture change. The UI uses Tauri/Leptos with Cargo/Trunk.
- The host MSRV is Rust 1.100.0. Respect `rust-toolchain.toml`; never silently raise the MSRV or downgrade host Rust for embedded tooling. Firmware may use its separately documented ESP toolchain.
- `rhizo-domain` is pure and obtains time through `Clock`; no direct wall-clock calls.
- `rhizo-mqtt-contract` and `rhizo-policy` remain `no_std` compatible and are the shared firmware-facing crates.
- While connected, the Edge owns high-level decisions and authoritative state. The device is always the final hardware safety boundary.
- Offline autonomy uses exactly one pure shared `rhizo_policy::evaluate_offline` implementation. Do not create simulator- or firmware-specific copies.
- The UI talks to Edge REST APIs, never directly to MQTT or hardware.
- Cloud is an optional append-only history sink and must never enter the local control path or originate commands.
- Persist-before-publish and documented transactional deduplication boundaries are safety properties, not implementation details.

## Essential safety and hardware

- Missing, stale, invalid or contradictory required inputs fail closed. Never
  add bypass, raw-pump, force or debug paths around the shared validator.
- Retain device hard limits, duration guard independent of MQTT, frequency and
  cumulative volume accounting, cooldown, deduplication and conservative restart.
- Before hardware work read the local inventory in full and follow local status
  checks. Verify physical wiring, polarity, supply voltages, logic levels and
  isolation before power. Inventory is not proof of actual connections.
- Keep pump power disabled until startup/reset/watchdog and control-signal safety
  have been checked on the actual driver. Establish reliable stop/power disconnect.
- Calibrate small doses into a cup before supervised plant watering. Missing
  sensor or communication cannot permit uncontrolled watering. Do not fake
  required tank/leak health. No unattended watering until essential fail-safes
  have been demonstrated. Scope claims as prototype tested, not production validated.

## Proportional testing

Follow [testing policy](docs/testing/strategy.md). Run relevant unit tests and a
basic integration smoke; add tests for important behavior, regressions and
realistic failures. Use real Mosquitto/SQLite for integration claims; enable
`RHIZO_REQUIRE_BROKER=1` for selected broker tests so absence is not a pass.
Use the separate `firmware/node-app` host suite for firmware application logic.

Do not routinely rerun full suites, mutation campaigns, endurance/HIL matrices
or power profiling. Broaden only for a relevant change, failure or concrete
risk. Preserve existing CI and lint constraints; CI coverage is not a command
to repeat every job locally. SQLx changes use the documented prepare/check flow.
Documentation-only changes need `cargo run -p rhizo-docscheck` and a diff review.

Mark only demonstrated acceptance criteria complete. Deferred scope stays open
and linked from the roadmap. Keep documentation concise, files LF, toolchains
pinned and local inventory/instructions untracked. Do not bulk regenerate issues.

## Git rules

Read-only Git inspection and requested working-tree edits are allowed. Preserve unrelated user changes. Unless the user explicitly requests it, do not run `git commit`, `git push`, `git tag`, `git rebase`, `git reset --hard`, `git cherry-pick`, force-push, create a PR/release, or rewrite history.
