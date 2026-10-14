#!/usr/bin/env bash
#
# Reports whether the ESP32-C3 development board is connected and ready from
# this WSL2 or Linux environment. The first diagnostic to run when unsure
# whether the real board is available; on native Windows use
# scripts/esp-status.ps1 instead.
#
#     ./scripts/esp-status.sh
#     ./scripts/esp-status.sh --json
#
# Finds the board's Silicon Labs CP210x / CP2102N bridge (VID:PID 10c4:ea60)
# with `espflash list-ports` rather than assuming /dev/ttyUSB0, checks that this
# user can open it, and reads the chip with `espflash board-info`.
#
# Non-destructive: it never flashes, erases, or runs usbipd. `board-info` does
# pulse the reset line, so firmware running on the board restarts, exactly as it
# does whenever espflash connects.
#
# Exit codes (shared with esp-status.ps1):
#    0  READY                 board verified, usable from here now
#   10  ESPFLASH MISSING      espflash not found
#   11  NOT CONNECTED         no matching bridge visible (in WSL: not attached)
#   12  PERMISSION ERROR      the tty exists but this user cannot open it
#   14  WRONG DEVICE          the bridge answers, but the chip is not an ESP32-C3
#   15  BOARD NOT RESPONDING  the port opened, but the chip did not answer
#   16  MULTIPLE DEVICES      more than one matching bridge; refuses to guess
#   17  PORT BUSY             another process holds the tty
#
# Inside an agent sandbox the serial device and supplementary groups can be
# hidden, which reads as NOT CONNECTED or PERMISSION ERROR. Re-run outside it
# before concluding anything about the hardware.

set -u

json=0
case "${1:-}" in
    --json) json=1 ;;
    -h|--help) sed -n '3,/^$/{s/^# \{0,1\}//;p}' "$0"; exit 0 ;;
    "") ;;
    *) echo "usage: $0 [--json]" >&2; exit 2 ;;
esac

environment="" ; espflash_version="" ; serial_port="" ; usb_uart=""
chip="" ; revision="" ; flash_size="" ; crystal_frequency=""
wifi="" ; ble="" ; secure_boot="" ; flash_encryption=""
permissions_ok="" ; device_owner="" ; device_mode=""

# JSON string, or null when empty.
js() {
    if [ -z "$1" ]; then printf 'null'; return; fi
    local s=$1
    s=${s//\\/\\\\}; s=${s//\"/\\\"}; s=${s//$'\t'/\\t}; s=${s//$'\n'/\\n}; s=${s//$'\r'/}
    printf '"%s"' "$s"
}
# JSON boolean, or null when empty.
jb() { if [ -z "$1" ]; then printf 'null'; else printf '%s' "$1"; fi; }

finish() {
    local state=$1 code=$2 message=$3 hint=${4:-}
    local ready=false; [ "$code" -eq 0 ] && ready=true

    if [ "$json" -eq 1 ]; then
        cat <<EOF
{
  "environment": $(js "$environment"),
  "ready": $ready,
  "status": $(js "$state"),
  "exit_code": $code,
  "espflash_version": $(js "$espflash_version"),
  "serial_port": $(js "$serial_port"),
  "usb_uart": $(js "$usb_uart"),
  "vid_pid": "10c4:ea60",
  "device_owner": $(js "$device_owner"),
  "device_mode": $(js "$device_mode"),
  "chip": $(js "$chip"),
  "revision": $(js "$revision"),
  "flash_size": $(js "$flash_size"),
  "crystal_frequency": $(js "$crystal_frequency"),
  "wifi": $(jb "$wifi"),
  "ble": $(jb "$ble"),
  "secure_boot": $(js "$secure_boot"),
  "flash_encryption": $(js "$flash_encryption"),
  "permissions_ok": $(jb "$permissions_ok"),
  "message": $(js "$message"),
  "hint": $(js "$hint")
}
EOF
        exit "$code"
    fi

    row() { [ -n "$2" ] && printf '%-18s%s\n' "$1:" "$2"; return 0; }
    echo "ESP32-C3 status"
    echo "---------------"
    row "Environment" "$environment"
    row "espflash" "$espflash_version"
    row "Serial" "$serial_port"
    row "USB-UART" "$usb_uart"
    if [ -n "$permissions_ok" ]; then
        if [ "$permissions_ok" = true ]; then p=OK; else p=DENIED; fi
        row "Permissions" "$p ($device_owner $device_mode)"
    fi
    if [ -n "$chip" ]; then row "Chip" "$chip${revision:+ rev $revision}"; fi
    row "Crystal" "$crystal_frequency"
    row "Flash" "$flash_size"
    if [ -n "$wifi" ]; then
        if [ "$wifi" = true ] && [ "$ble" = true ]; then row "Wi-Fi/BLE" yes
        else row "Wi-Fi/BLE" "Wi-Fi $wifi, BLE $ble"; fi
    fi
    row "Secure Boot" "$secure_boot"
    row "Flash Encryption" "$flash_encryption"
    echo
    echo "$state: $message"
    [ -n "$hint" ] && echo "$hint"
    exit "$code"
}

strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

# Environment ---------------------------------------------------------------
os_name=$( (. /etc/os-release 2>/dev/null && echo "$NAME ${VERSION_ID:-}") || uname -s)
os_name=${os_name% }
in_wsl=0
if grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
    in_wsl=1
    environment="WSL2 / $os_name"
else
    environment="Linux / $os_name"
fi

# 1. espflash ---------------------------------------------------------------
espflash=$(command -v espflash || true)
if [ -z "$espflash" ] && [ -x "$HOME/.cargo/bin/espflash" ]; then
    espflash="$HOME/.cargo/bin/espflash"
fi
if [ -z "$espflash" ]; then
    finish "ESPFLASH MISSING" 10 "espflash is not on PATH." \
        "Install it with: cargo install espflash --locked (Linux also needs libudev-dev)"
fi
espflash_version=$("$espflash" --version 2>/dev/null </dev/null | head -n1 | sed 's/^espflash[[:space:]]*//')

# 2. Find the bridge --------------------------------------------------------
# espflash prints "PORT  PID:VID  MANUFACTURER  PRODUCT"; accept either order.
mapfile -t hits < <("$espflash" list-ports --skip-update-check </dev/null 2>/dev/null | strip_ansi |
    awk 'tolower($2) == "ea60:10c4" || tolower($2) == "10c4:ea60"')

if [ "${#hits[@]}" -gt 1 ]; then
    ports=$(printf '%s\n' "${hits[@]}" | awk '{print $1}' | paste -sd, -)
    finish "MULTIPLE DEVICES" 16 "${#hits[@]} CP210x serial devices found; refusing to guess." \
        "Devices: $ports. Unplug the extra one."
fi
if [ "${#hits[@]}" -eq 0 ]; then
    others=$(ls /dev/ttyUSB* /dev/ttyACM* 2>/dev/null | paste -sd, -)
    if [ -n "$others" ]; then
        detail="Serial devices present ($others), but none is a CP210x (10c4:ea60)."
    else
        detail="No /dev/ttyUSB* or /dev/ttyACM* device exists."
    fi
    if [ "$in_wsl" -eq 1 ]; then
        finish "NOT CONNECTED" 11 "ESP32 USB bridge is not visible inside WSL. $detail" \
            "From Windows PowerShell, in the repository: .\\scripts\\esp-wsl-attach.ps1"
    fi
    finish "NOT CONNECTED" 11 "ESP32 USB bridge is not visible. $detail" \
        "Check the USB cable (it must be a data cable) and that the board is powered."
fi

line=${hits[0]}
serial_port=$(awk '{print $1}' <<<"$line")
usb_uart=$(awk '{ $1=""; $2=""; sub(/^[[:space:]]+/, ""); print }' <<<"$line")

# 3. Permissions ------------------------------------------------------------
device_owner=$(stat -c '%U:%G' "$serial_port" 2>/dev/null || true)
device_mode=$(stat -c '%a' "$serial_port" 2>/dev/null || true)
group=${device_owner#*:}
if [ -r "$serial_port" ] && [ -w "$serial_port" ]; then
    permissions_ok=true
else
    permissions_ok=false
    me=$(id -un)
    if id -nG | tr ' ' '\n' | grep -qx "$group"; then
        hint="You are in '$group' but still cannot open it; check the mode ($device_mode)."
    elif getent group "$group" | cut -d: -f4 | tr ',' '\n' | grep -qx "$me"; then
        hint="You were added to '$group' after this session started. Restart the shell (in WSL: 'wsl --shutdown' from Windows, then reopen)."
    else
        hint="Add yourself to '$group': sudo usermod -aG $group $me, then restart the shell (in WSL: 'wsl --shutdown')."
    fi
    finish "PERMISSION ERROR" 12 "$serial_port exists ($device_owner $device_mode) but $me cannot open it." "$hint"
fi

# 4. board-info -------------------------------------------------------------
raw=$(timeout 60 "$espflash" board-info --port "$serial_port" --non-interactive --skip-update-check \
    </dev/null 2>&1)
rc=$?
# The MAC is dropped here, before anything can print or parse it.
info=$(strip_ansi <<<"$raw" | grep -v 'MAC address')
unset raw
if [ "$rc" -eq 124 ]; then
    finish "BOARD NOT RESPONDING" 15 "espflash board-info on $serial_port did not finish within 60 s." \
        "Replug the board; if it persists, hold BOOT while pressing RESET and retry."
fi
if [ "$rc" -ne 0 ]; then
    if grep -qiE 'busy|in use|lock' <<<"$info"; then
        finish "PORT BUSY" 17 "$serial_port is held by another process." \
            "Close any serial monitor (espflash monitor, screen, minicom) and retry."
    fi
    if grep -qi 'permission denied' <<<"$info"; then
        permissions_ok=false
        finish "PERMISSION ERROR" 12 "Cannot open $serial_port: permission denied." ""
    fi
    last=$(grep -v '^[[:space:]]*$' <<<"$info" | tail -n1)
    finish "BOARD NOT RESPONDING" 15 "The port opened but the chip did not answer: $last" \
        "Replug the board; if it persists, hold BOOT while pressing RESET and retry."
fi

field() { sed -n "s/^[[:space:]]*$1:[[:space:]]*//p" <<<"$info" | head -n1 | sed 's/[[:space:]]*$//'; }

chip_line=$(field 'Chip type')
chip_raw=${chip_line%% *}
revision=$(sed -n 's/.*(revision v\{0,1\}\([^)]*\)).*/\1/p' <<<"$chip_line")
crystal_frequency=$(field 'Crystal frequency')
flash_size=$(field 'Flash size' | sed 's/\([0-9]\)\([KMG]B\)$/\1 \2/')
features=$(field 'Features')
if [ -n "$features" ]; then
    if grep -qiE 'wifi|wi-fi' <<<"$features"; then wifi=true; else wifi=false; fi
    if grep -q 'BLE' <<<"$features"; then ble=true; else ble=false; fi
fi
secure_boot=$(field 'Secure Boot' | tr '[:upper:]' '[:lower:]')
flash_encryption=$(field 'Flash Encryption' | tr '[:upper:]' '[:lower:]')
if [ -n "$chip_raw" ]; then
    chip=$(tr '[:lower:]' '[:upper:]' <<<"$chip_raw" | sed 's/^ESP32\([A-Z]\)/ESP32-\1/')
fi

if [ "$chip_raw" != "esp32c3" ]; then
    finish "WRONG DEVICE" 14 "The CP210x bridge answers, but the chip is '${chip_raw:-unknown}', not an ESP32-C3." \
        "Another ESP board may be plugged in instead of the Rhizo ESP32-C3."
fi

finish "READY" 0 "ESP32-C3 is available for development."
