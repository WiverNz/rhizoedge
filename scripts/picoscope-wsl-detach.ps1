# Discover the current PicoScope BUSID; never elevate, force-bind or unbind.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'picoscope-wsl-common.ps1')

Assert-Usbipd
$dev = Get-SinglePicoDevice

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

Write-Host "Detached BUSID $($dev.BusId); the PicoScope is back on Windows." -ForegroundColor Green
exit 0
