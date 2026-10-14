<#
.SYNOPSIS
    Attach the ESP32-C3 development board's USB-UART bridge to WSL2.

.DESCRIPTION
    For Windows hosts that flash and monitor the ESP32-C3 board from WSL2 using
    usbipd-win. Finds the Silicon Labs CP210x / CP2102N bridge (VID:PID
    10c4:ea60) in `usbipd list`, discovers its current BUSID, and runs
    `usbipd attach --wsl --busid <BUSID>`.

    - No matching device: error, exit 1.
    - More than one matching device: lists them, refuses to guess, exit 1.
    - Already attached: says so, exit 0.
    - Not shared (never bound): prints the one-time Administrator command
      `usbipd bind --busid <BUSID>` and exits 1. It never elevates itself.

    Normal use needs no Administrator rights. Re-run after a WSL restart,
    `wsl --shutdown`, or unplugging the board: attachment does not survive them.

.EXAMPLE
    .\scripts\esp-wsl-attach.ps1
#>

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'esp-wsl-common.ps1')

Assert-Usbipd
$dev = Get-SingleEspDevice

Write-Host "Detected: $($dev.Device)"
Write-Host "  BUSID $($dev.BusId)   VID:PID $($dev.VidPid)   state: $($dev.State)"

if ($dev.State -match 'Attached') {
    Write-Host "Already attached to WSL (BUSID $($dev.BusId)); nothing to do." -ForegroundColor Green
    Write-Host 'In WSL, check: ls -l /dev/ttyUSB0'
    exit 0
}

if ($dev.State -match 'Not shared') {
    Write-Host 'ERROR: the device is not shared (bound) with usbipd yet.' -ForegroundColor Red
    Write-Host 'Run this once from an Administrator PowerShell, then re-run this script:'
    Write-Host "  usbipd bind --busid $($dev.BusId)"
    exit 1
}

Write-Host "Running: usbipd attach --wsl --busid $($dev.BusId)"
$result = Invoke-Usbipd attach --wsl --busid $dev.BusId
$out = $result.Output
$code = $result.ExitCode
$out | ForEach-Object { Write-Host "  $_" }

if ($code -ne 0) {
    # A race with another attach, or a state usbipd reports differently.
    if (($out -join "`n") -match 'already attached') {
        Write-Host 'Device was already attached to WSL; treating as success.' -ForegroundColor Green
        Write-Host 'In WSL, check: ls -l /dev/ttyUSB0'
        exit 0
    }
    Write-Host "ERROR: usbipd attach failed (exit $code)." -ForegroundColor Red
    exit $code
}

Write-Host "Attached BUSID $($dev.BusId) to WSL." -ForegroundColor Green
Write-Host 'In WSL, check: ls -l /dev/ttyUSB0   (or: espflash list-ports)'
exit 0
