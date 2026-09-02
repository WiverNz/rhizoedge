#!/usr/bin/env bash
#
# Runs the M8 assembled-system scenario suite, or the scenarios named as
# arguments, and exits with the runner's status.
#
#     ./scripts/run-scenarios.sh
#     ./scripts/run-scenarios.sh --scenario scenario_first_demo
#     ./scripts/run-scenarios.sh --list
#
# # Why this is not `up --abort-on-container-exit --exit-code-from scenario-runner`
#
# Because that command cannot work here, and the reason is structural rather
# than incidental. `--abort-on-container-exit` tears the project down the moment
# **any** container exits — and stopping containers is what a third of these
# scenarios *do*. SCEN-051 kills the edge between a publish and the row that
# records it; SCEN-012 restarts the broker; SCEN-060 stops the cloud for a whole
# scenario; and the harness stops the edge and the simulator between every pair
# of scenarios to give each one a clean database. Under that flag the first such
# stop ends the run, and the runner is SIGKILLed mid-scenario with exit 137 —
# which looks like a scenario failure and is not one.
#
# So the topology is brought up and waited on first, and the runner is a
# one-shot container against it. That is still one command, it still exits
# non-zero on any failure, and it can no longer confuse "a scenario stopped a
# service" with "a service died".
#
# Diagnostics are collected here rather than in CI so that a local failure and a
# CI failure produce the same artefacts (F-080-15).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

compose=(docker compose
    -f deploy/docker-compose.yml
    -f deploy/docker-compose.test.yml)

artifacts="deploy/artifacts"
mkdir -p "$artifacts"

teardown() {
    "${compose[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
}

echo "==> building every image, including the profiled runner"
# `--profile runner` explicitly, because `up --build` builds only the services
# it starts and the runner is deliberately not one of them. Without this the
# runner keeps whatever image it was last built with, and a scenario edited a
# moment ago runs in its previous form — which is indistinguishable from a
# scenario that did not take effect, and cost an afternoon to see once already.
"${compose[@]}" --profile runner build

echo "==> starting the accelerated topology"
# `--wait` blocks on the health gates rather than on a sleep, so the runner's
# own startup checks are the first thing that can fail.
"${compose[@]}" up -d --wait

echo "==> running scenarios"
set +e
# `--no-deps`: the topology is already up and healthy, and letting `run`
# recreate dependencies would restart the very services a scenario is about to
# stop on purpose.
"${compose[@]}" run --rm --no-deps scenario-runner "$@"
status=$?
set -e

if [ "$status" -ne 0 ]; then
    echo "==> a scenario failed; collecting diagnostics into $artifacts"
    "${compose[@]}" logs --no-color >"$artifacts/compose.log" 2>&1 || true
fi

teardown
exit "$status"
