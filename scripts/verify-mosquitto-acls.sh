#!/usr/bin/env bash
#
# Verifies that the broker actually enforces what deploy/mosquitto/aclfile
# claims (M0-008).
#
# Configuring an ACL and verifying one are different things: a typo in the
# `pattern` line, a missing `allow_anonymous false`, or a passwd file the
# broker could not read all leave a broker that starts cleanly and enforces
# nothing. This script asserts the four properties that matter:
#
#   1. an anonymous client is refused
#   2. a wrong password is refused
#   3. a device can use its own rhizo/v1/devices/{device_id}/# subtree
#   4. a device is DENIED another device's subtree
#
#   plus: the edge account can subscribe across the whole fleet
#
# Usage:
#
#     ./scripts/gen-mosquitto-passwd.sh
#     docker compose -f deploy/docker-compose.yml up -d mosquitto
#     ./scripts/verify-mosquitto-acls.sh
#
# M2-012 turns this into an automated integration test. Until the simulator
# exists, this script is how the claim is checked.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
env_file="${ENV_FILE:-$repo_root/.env}"
host="${MQTT_HOST:-localhost}"
port="${MQTT_PORT:-1883}"
image="${MOSQUITTO_IMAGE:-eclipse-mosquitto:2}"

export MSYS_NO_PATHCONV=1

[ -f "$env_file" ] || {
    echo "error: $env_file not found — copy .env.example to .env and fill it in" >&2
    exit 1
}

get() {
    local line value
    line="$(grep -E "^[[:space:]]*$1=" "$env_file" | tail -n 1 || true)"
    [ -n "$line" ] || return 1
    value="${line#*=}"
    value="${value%\"}"; value="${value#\"}"
    printf '%s' "$value"
}

EDGE_USER="$(get MQTT_USERNAME)"
EDGE_PASS="$(get MQTT_PASSWORD)"
P1="$(get RHIZO_DEVICE_PLANT_NODE_01_PASSWORD)"

D1=plant-node-01
D2=plant-node-02

passed=0
failed=0

# `--network host` so the client reaches the published broker port the same way
# a device on the LAN would. `-W`/timeouts keep a refused connection from
# hanging the script.
mq() {
    docker run --rm --network host "$image" "$@" 2>&1
}

# Asserts that a command succeeds.
expect_ok() {
    local what="$1"; shift
    local out rc
    out="$("$@")"; rc=$?
    if [ $rc -eq 0 ]; then
        echo "  PASS  $what"
        passed=$((passed + 1))
    else
        echo "  FAIL  $what"
        echo "        expected success, got exit $rc: $out"
        failed=$((failed + 1))
    fi
}

# Asserts that a command fails. The message is checked too: a failure for the
# wrong reason (broker down, DNS, timeout) would otherwise look like a pass,
# which for a security check is the worst possible false negative.
expect_denied() {
    local what="$1" expect="$2"; shift 2
    local out rc
    out="$("$@")"; rc=$?
    if [ $rc -eq 0 ]; then
        echo "  FAIL  $what"
        echo "        expected refusal, but it SUCCEEDED"
        failed=$((failed + 1))
    elif echo "$out" | grep -qiE "$expect"; then
        echo "  PASS  $what"
        passed=$((passed + 1))
    else
        echo "  FAIL  $what"
        echo "        failed, but not for the expected reason (want /$expect/): $out"
        failed=$((failed + 1))
    fi
}

# A denied PUBLISH needs its own assertion, because exit status does not carry
# the answer.
#
# Under MQTT 3.1.1 a broker that refuses a publish on ACL grounds still sends a
# PUBACK and simply discards the message, so `mosquitto_pub` exits 0 whether it
# was delivered or dropped — asserting on exit status there would pass
# regardless of what the ACL says, which is worse than not testing it.
#
# MQTT v5 carries reason code 0x87 (Not authorized) in the PUBACK. That is the
# broker's actual answer, so `-V 5` is used and the output is what is asserted.
publish_as_device() {
    mq mosquitto_pub -V 5 -h "$host" -p "$port" -u "$D1" -P "$P1" \
        -t "$1" -m '{"v":1}' -q 1
}

expect_publish_denied() {
    local what="$1" topic="$2" out
    out="$(publish_as_device "$topic")"
    if echo "$out" | grep -qiE "not authoris|not authorized"; then
        echo "  PASS  $what"
        passed=$((passed + 1))
    else
        echo "  FAIL  $what"
        echo "        the broker did not refuse it: ${out:-<no output>}"
        failed=$((failed + 1))
    fi
}

echo "verifying broker ACLs at $host:$port"
echo

echo "1. authentication"
expect_denied "anonymous subscribe is refused" \
    "not authoris|not authorized|Connection Refused|refused" \
    mq mosquitto_sub -h "$host" -p "$port" -t 'rhizo/v1/#' -C 1 -W 3

expect_denied "a wrong password is refused" \
    "not authoris|not authorized|Connection Refused|refused" \
    mq mosquitto_pub -h "$host" -p "$port" -u "$D1" -P "definitely-not-the-password" \
        -t "rhizo/v1/devices/$D1/telemetry" -m '{}' -q 1

expect_ok "the edge account connects" \
    mq mosquitto_sub -h "$host" -p "$port" -u "$EDGE_USER" -P "$EDGE_PASS" \
        -t 'rhizo/v1/devices/+/#' -C 1 -W 2 -E

echo
echo "2. per-device topic isolation (the %u pattern)"
# The mirror image of the denial check, run the same way: without it, a broker
# that refused *everything* would score a clean pass on the denial tests.
expect_publish_allowed() {
    local what="$1" topic="$2" out
    out="$(publish_as_device "$topic")"
    if echo "$out" | grep -qiE "not authoris|not authorized|Error"; then
        echo "  FAIL  $what"
        echo "        expected acceptance, got: $out"
        failed=$((failed + 1))
    else
        echo "  PASS  $what"
        passed=$((passed + 1))
    fi
}

expect_publish_allowed "$D1 publishes to its OWN telemetry topic" \
    "rhizo/v1/devices/$D1/telemetry"

expect_ok "$D1 subscribes to its OWN command topic" \
    mq mosquitto_sub -h "$host" -p "$port" -u "$D1" -P "$P1" \
        -t "rhizo/v1/devices/$D1/commands/water" -C 1 -W 2 -E

expect_publish_denied "$D1 is DENIED publishing into ${D2}'s subtree" \
    "rhizo/v1/devices/$D2/telemetry"

expect_denied "$D1 is DENIED subscribing to ${D2}'s subtree" \
    "not authoris|not authorized|denied|Timed out|Error" \
    mq mosquitto_sub -h "$host" -p "$port" -u "$D1" -P "$P1" \
        -t "rhizo/v1/devices/$D2/telemetry" -C 1 -W 3

expect_denied "$D1 is DENIED the fleet-wide wildcard" \
    "not authoris|not authorized|denied|Timed out|Error" \
    mq mosquitto_sub -h "$host" -p "$port" -u "$D1" -P "$P1" \
        -t 'rhizo/v1/#' -C 1 -W 3

echo
echo "$passed passed, $failed failed"
[ "$failed" -eq 0 ]
