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

# Accounts are collected first and hashed in ONE pass at the end, as
# tab-separated `user<TAB>password` records fed on stdin. Two reasons:
#
#   - `mosquitto_passwd -c` fails on a Windows or macOS bind mount with
#     "Unable to open file /config/passwd for writing. File exists." even when
#     it does not exist. The tool creates a scratch file with `O_EXCL` beside
#     the target and the host filesystem shim answers that call wrongly.
#     Building the file on the container's own filesystem and copying it into
#     `/config` once sidesteps the shim entirely.
#   - one container invocation instead of one per account is also much faster.
#
# A tab cannot occur in a base64 password, which is what ADR-012 §Credentials
# recommends, and no password ever reaches a process argument list or the host
# filesystem — the records are piped.
accounts=""
add_account() {
    local user="$1" pass="$2"
    accounts="${accounts}${user}${tab}${pass}${newline}"
    echo "  + $user"
}
tab="$(printf '\t')"
newline="$(printf '\nx')"; newline="${newline%x}"

edge_user="$(get MQTT_USERNAME || true)"
edge_pass="$(get MQTT_PASSWORD || true)"
device_ids="$(get DEVICE_IDS || true)"

[ -n "$edge_user" ] || die "MQTT_USERNAME is not set in $env_file"
reject_placeholder MQTT_PASSWORD "$edge_pass"

mkdir -p "$config_dir"
echo "generating $out_file from $(basename "$env_file")"

add_account "$edge_user" "$edge_pass"

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

# One hashing pass over every collected account, then the copy into the mount.
if [ -n "${MOSQUITTO_PASSWD:-}" ]; then
    rm -f "$out_file"
    printf '%s' "$accounts" | while IFS="$tab" read -r user pass; do
        [ -n "$user" ] || continue
        # `-c` creates; without it the file must already exist.
        if [ -f "$out_file" ]; then
            "$MOSQUITTO_PASSWD" -b "$out_file" "$user" "$pass"
        else
            "$MOSQUITTO_PASSWD" -c -b "$out_file" "$user" "$pass"
        fi
    done
else
    # Create the destination from the host before the container truncates it.
    # A Windows bind mount caches a negative lookup for a path deleted on the
    # host side and then answers the container's `O_CREAT` with ENOENT
    # ("can't create /config/passwd: nonexistent directory") for a directory
    # that plainly exists. Creating it here means the container only ever
    # truncates a file the mount already knows about.
    : > "$out_file" 2>/dev/null || true
    printf '%s' "$accounts" | docker run --rm -i \
        -v "$config_dir:/config" \
        "$image" sh -c '
            set -e
            rm -f /tmp/passwd
            while IFS="$(printf "\t")" read -r user pass; do
                [ -n "$user" ] || continue
                if [ -f /tmp/passwd ]; then
                    mosquitto_passwd -b /tmp/passwd "$user" "$pass"
                else
                    mosquitto_passwd -c -b /tmp/passwd "$user" "$pass"
                fi
            done
            # A redirect, not `cp`: BusyBox `cp` opens the destination with
            # `O_EXCL` and a Windows bind mount answers that with a spurious
            # EEXIST for a path that does not exist.
            cat /tmp/passwd > /config/passwd
        '
fi

[ -s "$out_file" ] || die "no accounts were written to $out_file"

# `mosquitto_passwd` creates the file 0600 owned by whoever ran it. In the
# Docker path that is root inside the container, and the broker runs as
# `mosquitto` (uid 1883) — so the file it just wrote would be one it cannot
# read: "Error: Unable to open pwfile". Handing it to `mosquitto` is therefore
# part of generating it, and has to happen inside the container, because the
# host user does not own a root-owned file.
#
# Both operations are allowed to fail. On a Windows or macOS bind mount the
# filesystem does not carry Unix ownership at all; the chown is a no-op there
# and the broker can read the file regardless.
if [ -n "${MOSQUITTO_PASSWD:-}" ]; then
    chmod 600 "$out_file" 2>/dev/null || true
else
    docker run --rm \
        -v "$config_dir:/config" \
        "$image" sh -c \
        'chown mosquitto:mosquitto /config/passwd 2>/dev/null || true
         chmod 600 /config/passwd 2>/dev/null || true'
fi

echo "wrote $out_file"
echo
echo "This file holds real credentials and is gitignored. Do not commit it."
