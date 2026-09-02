#!/bin/sh
#
# Resolves this container's device identity, then execs the simulator.
#
# ADR-012 makes the broker username the `device_id`, and the Mosquitto ACL's
# `pattern readwrite rhizo/v1/devices/%u/#` is what turns that into a real
# boundary. So every replica needs its *own* account: a shared id would make
# replicas collide on the broker, and a shared account would hand each of them
# fleet-wide rights and quietly make every isolation scenario vacuous.
#
# The simulator resolves its own username and password from the id
# (`RHIZO_DEVICE_<ID>_PASSWORD`), so this script only has to decide *which* id
# this container is, and everything else follows.
#
# # Finding the replica index
#
# `docker compose up --scale device-simulator=3` names containers
# `<project>-<service>-<n>` but sets each container's hostname to its own id, so
# `$HOSTNAME` carries no index. The number lives in a Compose *label*, which is
# not readable from inside the container.
#
# What is readable is the container's own address, and Docker's embedded DNS
# answers a reverse lookup of it with the container name. That is where the
# index comes from. Every failure falls back to replica 1, which is the
# single-instance case and the overwhelmingly common one — a device that
# refused to start because DNS was slow would be a worse outcome than a device
# that started as `plant-node-01`.
set -eu

if [ -n "${RHIZO_SIM_DEVICE_ID:-}" ]; then
    # An explicitly named node — the battery profile's `battery-node-01`. Never
    # scaled, so there is no index to derive.
    device_id="$RHIZO_SIM_DEVICE_ID"
else
    index=1
    address="$(busybox hostname -i 2>/dev/null | busybox cut -d' ' -f1 || true)"
    if [ -n "${address:-}" ]; then
        name="$(busybox nslookup "$address" 2>/dev/null |
            busybox sed -n 's/.*name = \([^ ]*\)/\1/p' | busybox head -n 1)"
        candidate="${name%%.*}"
        candidate="${candidate##*-}"
        case "$candidate" in
            '' | *[!0-9]*) ;;
            *) index="$candidate" ;;
        esac
    fi
    device_id="$(printf 'plant-node-%02d' "$index")"
fi

echo "device identity resolved: ${device_id}" >&2

exec /usr/local/bin/device-simulator \
    --device-id "$device_id" \
    --state-file "/var/lib/rhizo/${device_id}.state.json" \
    "$@"
