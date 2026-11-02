# Discover the current PicoScope BUSID; never elevate, force-bind or unbind.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'picoscope-wsl-common.ps1')

Assert-Usbipd
$dev = Get-SinglePicoDevice

Write-Host "Detected: $($dev.Device)"
Write-Host "  BUSID $($dev.BusId)   VID:PID $($dev.VidPid)   state: $($dev.State)"

if ($dev.State -match 'Attached') {
    Write-Host "Already attached to WSL (BUSID $($dev.BusId)); nothing to do." -ForegroundColor Green
    Write-Host 'In WSL, check: ./scripts/picoscope-status.sh'
    exit 0
}

if ($dev.State -match 'Not shared') {
    Write-Host 'ERROR: the device is not shared (bound) with usbipd yet.' -ForegroundColor Red
    Write-Host 'Run this once from an Administrator PowerShell, then re-run this script:'
    Write-Host "  usbipd bind --busid $($dev.BusId)"
    exit 18
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
        Write-Host 'In WSL, check: ./scripts/picoscope-status.sh'
        exit 0
    }
    Write-Host "ERROR: usbipd attach failed (exit $code)." -ForegroundColor Red
    # Preserve the ESP helper's conservative busy-device policy. The recorded
    # host bugcheck involved the ESP bridge, not this PicoScope. Never retry
    # a refused busy attach or bind with --force.
    if (($out -join "`n") -match 'used by Windows|Device busy') {
        Write-Host 'Windows still considers the device in use. Do not retry and do not use --force.'
        Write-Host 'Close any program using the PicoScope, unplug and replug the PicoScope, then run this again.'
    }
    exit $code
}

Write-Host "Attached BUSID $($dev.BusId) to WSL." -ForegroundColor Green
Write-Host 'In WSL, check: ./scripts/picoscope-status.sh'
exit 0
