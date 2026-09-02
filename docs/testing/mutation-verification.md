# Mutation verification of the safety suite (M8-013, M8-017)

**Milestone:** M8 · **PRD:** [PRD 080](../prd/080-end-to-end-test-environment.md)
§Testing strategy · **Issues:** [M8-013](../issues/M8/013-add-mutation-verification.md),
[M8-017](../issues/M8/017-implement-battery-and-sleep-scenarios.md)

## Why this document exists

A test suite that stays green when the safety logic is removed is decoration.
Every other document in this repository describes what the system *does*; this
one is the evidence that the tests would notice if it stopped.

The method is deliberately crude and deliberately manual. Each mutation removes
or inverts exactly one safety mechanism, the scenario suite runs against the
mutated build, and the named scenario must go **red**. Then the mutation is
reverted. There is no framework, no permanent variant of the codebase, and no
automation: maintaining seven broken forks of a safety-critical system would
cost more than it proves, and running the set once at milestone acceptance is
what converts the suite from *assumed* effective to *demonstrated* effective.

**A mutation that does not turn its scenario red is a finding**, not a curiosity.
It means the scenario is not testing what its name claims, and the scenario —
not the mutation — is what gets fixed.

## How to run one

**The mutation is applied in a throwaway worktree, never in the tree you work
in.** A mutation is a deliberate act of vandalism against a safety mechanism, and
the revert is what makes it survivable — so the revert must not be able to reach
anything except the mutation itself.

```bash
# One worktree per mutation, detached at the commit under test.
git worktree add --detach ../rhizoedge-mutation HEAD
cd ../rhizoedge-mutation

$EDITOR <the file named below>      # apply the mutation here, in the worktree

docker compose -p rhizo-mutation \
  -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  build edge-controller device-simulator
docker compose -p rhizo-mutation \
  -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  run --rm scenario-runner --scenario <the scenario named below>
# expect a non-zero exit and that scenario reported FAILED

docker compose -p rhizo-mutation \
  -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml down -v
cd -
git worktree remove --force ../rhizoedge-mutation
```

A fresh worktree is also what makes "the mutation must be the only change" true
by construction rather than by a `git stash list` that a reader can forget.

The explicit `-p rhizo-mutation` is not decoration. Compose derives its project
name from the directory, so a run from the worktree would otherwise adopt the
containers, networks, and volumes belonging to whatever project the directory
name happens to spell — naming it makes the isolation something chosen rather
than something inherited. `down -v` then disposes of the mutated build's state,
so the next mutation does not start against a database written by a build with a
safety mechanism removed.

## Never revert with a path

Reverting is the dangerous half of this procedure, and one form of it is
prohibited outright:

```bash
git checkout -- crates/      # NEVER. Discards every uncommitted change under
git restore crates/          # the path — the mutation and hours of unrelated
git checkout .               # work alike, with no confirmation and no undo.
```

`git checkout` cannot tell which change was the mutation. A path-wide revert in a
tree holding uncommitted work destroys all of it, and the loss is silent: the
command succeeds, prints nothing, and leaves a tree that looks deliberately
clean. That is exactly how it goes unnoticed until the next build.

This happened in this repository on 2026-09-02, during the first run of this
procedure. Recovery was possible only because the edits still existed in a
session transcript — which is luck, not a backup. Untracked files survived
untouched, because `git checkout` does not consider them; every *modified*
tracked file under `crates/` was lost.

The worktree above is why the question no longer arises: disposing of the
worktree disposes of the mutation, and the tree you work in was never a
participant.

**If a worktree is genuinely unavailable**, the mutation may be applied in place
— but the revert then names exactly one file, never a directory:

```bash
git diff > ../pre-mutation.patch             # uncommitted work, saved first
$EDITOR <the file named below>
# ... build, run, observe the failure ...
git checkout -- <the one file named below>   # one file. Never a path.
```

Better still, commit before starting. A mutation run against a tree with nothing
uncommitted in it has nothing to lose.

One mutation at a time. Two at once can mask each other — a build that cannot
water at all makes every watering scenario fail for the wrong reason, and the run
then proves nothing about either mechanism.

## The seven mutations

Six come from PRD 080 §Testing strategy; the seventh was added by ADR-018's
battery pass and is [M8-017](../issues/M8/017-implement-battery-and-sleep-scenarios.md)'s.

### 1 — Remove the leak check from the gate

**Mechanism:** SAFETY-003, the first refusal in the shared gate.
**Site:** `crates/domain/src/irrigation/gate.rs`, the
`LeakState::Detected => return Some(LockoutReason::Leak)` arm.
**Mutation:** delete the arm, so a detected leak falls through to the tank check.
**Must fail:** `scenario_leak` (SCEN-040).

The scenario floods the tray through the simulator control API, then calls
`POST /plants/{id}/water` and requires a **409** with the leak reason and
**nothing** on any `commands/*` topic. With the arm removed the gate permits the
dose and the MQTT spy sees a `command.water`.

### 2 — Use `device_time_ms` for staleness

**Mechanism:** SAFETY-005 — freshness is judged by the **edge's** `received_at`,
never by a timestamp the device chose.
**Site:** `crates/edge-controller/src/plant/mod.rs`, the freshness comparison
that reads `row.received_at`.
**Mutation:** compare against the sample's device timestamp instead.
**Must fail:** `scenario_stale_sensor` (SCEN-022).

The scenario withholds soil moisture with the `stale-soil` fault while the device
keeps reporting everything else. A device whose clock keeps advancing therefore
keeps *claiming* freshness it does not have, the lockout never appears, and the
scenario's "locked out after the staleness window" assertion fails.

### 3 — Make the outbox drain blocking

**Mechanism:** SAFETY-008 — a cloud outage cannot disable monitoring or control.
**Site:** `crates/edge-controller/src/main.rs`, where
`cloud::drain::run` is handed to the supervisor as its own task.
**Mutation:** `await` it inline before the control loop is spawned, so the drain
holds the startup path.
**Must fail:** `scenario_cloud_unavailable` (SCEN-060).

The scenario stops `cloud-api` for its whole duration and requires local
watering to proceed and `/health/ready` to answer **200**. A blocking drain makes
the edge stop making decisions the moment the cloud is unreachable, which is
precisely the coupling ADR-008 exists to forbid.

### 4 — Re-publish commands on restart

**Mechanism:** SAFETY-010 — an edge restart cannot replay a completed command.
**Site:** `crates/edge-controller/src/control/command.rs`,
`Commander::reconcile`'s `else` arm, which marks a still-live command as
awaiting and publishes nothing.
**Mutation:** publish the command again instead of marking it awaiting.
**Must fail:** `scenario_restart_mid_command` (SCEN-051).

The scenario arms the `fault-exit-after-command-publish` marker, lets the edge
die between the publish and the row that records it, restarts it, and asserts
**one** `command.water` across both process lifetimes and exactly one watering
event. The mutated build publishes a second one, the spy sees two, and the
plant is watered twice.

### 5 — Use a calendar day for the rolling cap

**Mechanism:** SAFETY-006 — the 24-hour cap is rolling and derived from rows.
**Site:** `crates/domain/src/irrigation/budget.rs`, `window_start`.
**Mutation:** return midnight of the current day rather than `now - 24h`.
**Must fail:** `scenario_full_watering_cycle` (SCEN-002), which is this suite's
carrier for the property SCEN-034 states.

A calendar window hands a plant a fresh allowance at midnight, so a run that
straddles it can deliver twice the cap. Under the accelerated clock a virtual
midnight arrives within the scenario, and the "never exceeds `max_daily_ml`"
assertion fails.

### 6 — Let the simulator skip the shared validator

**Mechanism:** the single actuation gate — the device applies the same
`validate_water_command` the edge does, so a bug in the edge cannot become water.
**Site:** `crates/device-simulator/src/command.rs`, the single
`validate_water_command(command, &guard)` call.
**Mutation:** accept unconditionally, ignoring the verdict.
**Must fail:** `scenario_leak` and `scenario_tank_empty` (the suite's carriers
for SCEN-032), and `crates/device-simulator/tests/single_actuation_path.rs`,
which fails at `cargo test` before the suite is even reached.

This is the mutation that proves the simulator is a *device* and not a mirror of
the edge's opinion. Both halves are recorded: a mutation caught by a unit test
as well as by the assembled system is caught twice, which is the intent.

### 7 — Publish immediately to a sleeping device

**Mechanism:** ADR-018's durable command intent — a command for a sleeping
device is held and minted at the wake, never published into the dark.
**Site:** `crates/edge-controller/src/control/intents.rs`, `route`, which
answers `Route::HoldForWake` for a sleeping device.
**Mutation:** return `Route::Immediate` regardless of reachability.
**Must fail:** `scenario_sleeping_manual_water` (SCEN-113) and
`scenario_sleeping_safety_refusal` (SCEN-114).

SCEN-113's central assertion is a negative one made with the MQTT spy: **nothing
appears on any `commands/*` topic while the device sleeps**. The mutated build
publishes at once, into a session that is not there to receive it, and the
command expires unseen — the exact failure the intent mechanism exists to
prevent. SCEN-114 fails for a second, independent reason: the gate is re-run at
*delivery*, so a leak that appeared during the sleep refuses the dose; a
build that published up front made its decision before the leak existed.

## Results

Run once at M8 acceptance, on the commit recorded in
[docs/reports/M8.md](../reports/M8.md).

| # | Mutation | Scenario that must fail | Outcome |
|---|---|---|---|
| 1 | Leak check removed from the gate | `scenario_leak` | _pending_ |
| 2 | `device_time_ms` used for staleness | `scenario_stale_sensor` | _pending_ |
| 3 | Outbox drain made blocking | `scenario_cloud_unavailable` | _pending_ |
| 4 | Commands re-published on restart | `scenario_restart_mid_command` | _pending_ |
| 5 | Calendar day used for the cap | `scenario_full_watering_cycle` | _pending_ |
| 6 | Simulator skips the shared validator | `scenario_leak` | _pending_ |
| 7 | Immediate publish to a sleeping device | `scenario_sleeping_manual_water` | _pending_ |

## What this does not prove

The set is a spot check on seven mechanisms, not a coverage measure. It says
nothing about the mechanisms it does not touch, and a scenario that passes here
can still be weak in ways no mutation in this table would reveal. Its value is
narrow and real: for these seven, the suite demonstrably notices.
