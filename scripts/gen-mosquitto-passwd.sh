#!/usr/bin/env bash
#
# Generates deploy/mosquitto/passwd from .env.
#
# The broker refuses to start without this file, and the file is gitignored
# because it holds real credentials (ADR-011 §Secrets). Generating it is
# therefore part of first-run setup:
#
#     cp .env.example .env    # fill in the passwords
#     ./scripts/gen-mosquitto-passwd.sh
#     docker compose -f deploy/docker-compose.yml up -d mosquitto
#
# Accounts created:
#
#   - $MQTT_USERNAME          the edge control plane, `readwrite rhizo/v1/#`
#   - one per id in $DEVICE_IDS, each confined by the ACL's `%u` pattern to
#     `rhizo/v1/devices/<id>/#` (ADR-012)
#
# `mosquitto_passwd` runs inside the same `eclipse-mosquitto:2` image the
# broker uses, so no local install is needed and the hash format cannot drift
# from what the broker expects. Set MOSQUITTO_PASSWD to a local binary to skip
# Docker entirely.
#
# Re-running is safe: the file is rebuilt from scratch, so its contents depend
# only on .env.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
env_file="${ENV_FILE:-$repo_root/.env}"
config_dir="$repo_root/deploy/mosquitto"
out_file="$config_dir/passwd"
image="${MOSQUITTO_IMAGE:-eclipse-mosquitto:2}"

# Git Bash rewrites `/config` in a docker argument into a Windows path unless
# this is set. Harmless everywhere else.
export MSYS_NO_PATHCONV=1

die() {
    echo "error: $*" >&2
    exit 1
}

[ -f "$env_file" ] || die "$env_file not found — copy .env.example to .env and fill it in"

# Read .env as data. `source`ing a config file would execute whatever a stray
# backtick in a password happened to contain.
get() {
    local key="$1" line value
    line="$(grep -E "^[[:space:]]*${key}=" "$env_file" | tail -n 1 || true)"
    [ -n "$line" ] || return 1
    value="${line#*=}"
    value="${value%\"}"; value="${value#\"}"
    value="${value%\'}"; value="${value#\'}"
    printf '%s' "$value"
}

reject_placeholder() {
    case "$2" in
        change-me*|"") die "$1 is unset or still the placeholder from .env.example" ;;
    esac
}

# Adds one account. `-c` on the first call creates and truncates the file,
# which is what makes the whole script idempotent.
add_account() {
    local user="$1" pass="$2" create="${3:-}"
    if [ -n "${MOSQUITTO_PASSWD:-}" ]; then
        "$MOSQUITTO_PASSWD" ${create:+-c} -b "$out_file" "$user" "$pass"
    else
        docker run --rm \
            -v "$config_dir:/config" \
            "$image" mosquitto_passwd ${create:+-c} -b /config/passwd "$user" "$pass"
    fi
    echo "  + $user"
}

edge_user="$(get MQTT_USERNAME || true)"
edge_pass="$(get MQTT_PASSWORD || true)"
device_ids="$(get DEVICE_IDS || true)"

[ -n "$edge_user" ] || die "MQTT_USERNAME is not set in $env_file"
reject_placeholder MQTT_PASSWORD "$edge_pass"

mkdir -p "$config_dir"
echo "generating $out_file from $(basename "$env_file")"

add_account "$edge_user" "$edge_pass" create

if [ -n "$device_ids" ]; then
    IFS=',' read -r -a ids <<<"$device_ids"
    for id in "${ids[@]}"; do
        id="$(echo "$id" | tr -d '[:space:]')"
        [ -n "$id" ] || continue

        # `plant-node-01` -> RHIZO_DEVICE_PLANT_NODE_01_PASSWORD
        var="RHIZO_DEVICE_$(echo "$id" | tr '[:lower:]-' '[:upper:]_')_PASSWORD"
        pass="$(get "$var" || true)"
        [ -n "$pass" ] || die "$var is not set in $env_file (needed for device $id)"
        reject_placeholder "$var" "$pass"

        add_account "$id" "$pass"
    done
fi

chmod 600 "$out_file" 2>/dev/null || true

echo "wrote $out_file"
echo
echo "This file holds real credentials and is gitignored. Do not commit it."
