#!/usr/bin/env bash
#
# Writes a .env full of throwaway credentials, for a development machine or CI.
#
#     ./scripts/gen-dev-env.sh
#     ./scripts/gen-mosquitto-passwd.sh
#     docker compose -f deploy/docker-compose.yml up --build -d
#
# # This is not how a deployment gets its secrets
#
# ADR-011 §Secrets puts the real values in the environment layer, entered by a
# person and never committed. What this replaces is the tedium of doing that for
# a throwaway topology: it copies `.env.example` and replaces every
# `change-me-*` placeholder with 24 random bytes, so a fresh clone can be
# running in two commands instead of hand-editing seven passwords.
#
# It refuses to overwrite an existing `.env`, because the one thing worse than
# typing seven passwords is losing the seven you already typed.
#
# # The two edge passwords have to match
#
# `MQTT_PASSWORD` is the account `scripts/gen-mosquitto-passwd.sh` creates in
# the broker; `RHIZO_EDGE__MQTT__PASSWORD` is the one the edge connects with.
# They are separate keys because they belong to separate layers — the broker's
# account list and the edge's own configuration — and if they disagree the edge
# is refused with `NotAuthorized` and nothing else explains why. Generating them
# independently is the obvious mistake, so this generates one value and writes
# it to both.
#
# # Why it reads DEVICE_IDS rather than naming devices
#
# Both CI workflows used to spell the device password variables out. Adding a
# third device to `DEVICE_IDS` then broke them: `gen-mosquitto-passwd.sh`
# refuses a placeholder, correctly, and the jobs failed before anything was
# built. One list, read at the point of use, cannot drift from itself.
#
# Bash and coreutils only, like `gen-mosquitto-passwd.sh` beside it. First-run
# setup should need nothing a fresh clone does not already assume.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target="${ENV_FILE:-.env}"

if [ -e "$target" ] && [ "${FORCE:-}" != "1" ]; then
    echo "error: $target already exists; set FORCE=1 to overwrite it" >&2
    exit 1
fi

# 24 random bytes, base64 — what ADR-012 §Credentials recommends. `+` and `/`
# are dropped so a value is safe to carry through a `sed` replacement below.
secret() {
    head -c 24 /dev/urandom | base64 | tr -d '=+/'
}

filled=0
: >"$target"
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        \#*|"") printf '%s\n' "$line" >>"$target" ;;
        *change-me*)
            key="${line%%=*}"
            if [ "$key" = "$line" ]; then
                # A comment mentioning the placeholder, not an assignment.
                printf '%s\n' "$line" >>"$target"
            else
                printf '%s=%s\n' "$key" "$(secret)" >>"$target"
                filled=$((filled + 1))
            fi
            ;;
        *) printf '%s\n' "$line" >>"$target" ;;
    esac
done <.env.example
echo "filled $filled placeholder(s)"

get() {
    local key="$1" line
    line="$(grep -E "^${key}=" "$target" | tail -n 1 || true)"
    printf '%s' "${line#*=}"
}

edge_password="$(get MQTT_PASSWORD)"
[ -n "$edge_password" ] || {
    echo "error: MQTT_PASSWORD is missing from .env.example" >&2
    exit 1
}
sed -i "s|^RHIZO_EDGE__MQTT__PASSWORD=.*|RHIZO_EDGE__MQTT__PASSWORD=${edge_password}|" "$target"

# Every id in DEVICE_IDS needs the variable the simulator will look for. Saying
# so here, where the file is written, beats finding out from a broker refusal.
device_ids="$(get DEVICE_IDS)"
if [ -n "$device_ids" ]; then
    missing=""
    IFS=',' read -r -a ids <<<"$device_ids"
    for id in "${ids[@]}"; do
        id="$(echo "$id" | tr -d '[:space:]')"
        [ -n "$id" ] || continue
        var="RHIZO_DEVICE_$(echo "$id" | tr '[:lower:]-' '[:upper:]_')_PASSWORD"
        [ -n "$(get "$var")" ] || missing="$missing $var"
    done
    if [ -n "$missing" ]; then
        echo "error: DEVICE_IDS names devices with no password variable:$missing" >&2
        exit 1
    fi
    echo "device accounts: $device_ids"
fi

chmod 600 "$target" 2>/dev/null || true
echo "wrote $target"
echo
echo "These are throwaway credentials for a local or CI topology."
echo "This file is gitignored. Do not commit it, and do not deploy with it."
