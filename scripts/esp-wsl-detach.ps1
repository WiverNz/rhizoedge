<#
.SYNOPSIS
    Detach the ESP32-C3 development board's USB-UART bridge from WSL2.

.DESCRIPTION
    For Windows hosts that use the ESP32-C3 board from WSL2 via usbipd-win.
    Finds the Silicon Labs CP210x / CP2102N bridge (VID:PID 10c4:ea60) in
    `usbipd list`, discovers its current BUSID, and runs
    `usbipd detach --busid <BUSID>`, returning the serial port to Windows.

    - No matching device: error, exit 1.
    - More than one matching device: lists them, refuses to guess, exit 1.
    - Not attached: says so, exit 0.

    Needs no Administrator rights. The device stays bound (shared), so a later
    esp-wsl-attach.ps1 needs no Administrator rights either.

.EXAMPLE
    .\scripts\esp-wsl-detach.ps1
#>

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'esp-wsl-common.ps1')

Assert-Usbipd
$dev = Get-SingleEspDevice

Write-Host "Detected: $($dev.Device)"
Write-Host "  BUSID $($dev.BusId)   VID:PID $($dev.VidPid)   state: $($dev.State)"

if ($dev.State -notmatch 'Attached') {
    Write-Host "Not attached to WSL (state: $($dev.State)); nothing to detach." -ForegroundColor Green
    exit 0
}

Write-Host "Running: usbipd detach --busid $($dev.BusId)"
$result = Invoke-Usbipd detach --busid $dev.BusId
$out = $result.Output
$code = $result.ExitCode
$out | ForEach-Object { Write-Host "  $_" }

if ($code -ne 0) {
    if (($out -join "`n") -match 'not attached|already detached') {
        Write-Host 'Device was already detached; treating as success.' -ForegroundColor Green
        exit 0
    }
    Write-Host "ERROR: usbipd detach failed (exit $code)." -ForegroundColor Red
    exit $code
}

Write-Host "Detached BUSID $($dev.BusId); the board is back on Windows (normally COM5)." -ForegroundColor Green
exit 0
