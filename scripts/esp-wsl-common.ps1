# Shared lookup for esp-wsl-attach.ps1 and esp-wsl-detach.ps1. Dot-sourced by
# both; running it directly does nothing.
#
# For Windows hosts that develop against the ESP32-C3 board from WSL2, with
# usbipd-win (https://github.com/dorssel/usbipd-win) installed. Works on Windows
# PowerShell 5.1 and PowerShell 7 with no modules.
#
# Finds the board's USB-UART bridge in `usbipd list` by VID:PID, never by
# BUSID: the BUSID depends on which physical port the cable is in and changes
# when it moves, while the VID:PID belongs to the bridge chip. 10c4:ea60 is the
# Silicon Labs CP210x family, CP2102N included, which is what the development
# board carries. A board with a different bridge (CH340, native USB-JTAG) needs
# a different identifier here.

$EspVidPid = '10c4:ea60'
$EspDescriptionPattern = 'CP210|CP2102N|USB to UART Bridge'

# Runs usbipd and returns @{ Output = [string[]]; ExitCode = [int] }.
#
# usbipd writes progress ("usbipd: info: Using WSL distribution ...") to stderr.
# Windows PowerShell 5.1 turns redirected native stderr into ErrorRecords, and
# under the callers' $ErrorActionPreference = 'Stop' the first one aborts the
# script mid-attach. Success is therefore judged by the exit code alone.
function Invoke-Usbipd {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & usbipd @args 2>&1 | ForEach-Object { "$_" }
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $saved
    }
    return @{ Output = @($output); ExitCode = $code }
}

function Assert-Usbipd {
    if (-not (Get-Command usbipd -ErrorAction SilentlyContinue)) {
        Write-Host 'ERROR: usbipd was not found on PATH. Install usbipd-win first.' -ForegroundColor Red
        exit 2
    }
}

# Returns every row of the "Connected:" section matching the ESP bridge, as
# objects with BusId, VidPid, Device, and State.
function Find-EspDevice {
    $result = Invoke-Usbipd list
    $output = $result.Output
    if ($result.ExitCode -ne 0) {
        Write-Host 'ERROR: `usbipd list` failed:' -ForegroundColor Red
        $output | ForEach-Object { Write-Host "  $_" }
        exit 2
    }

    $inConnected = $false
    $hits = @()
    foreach ($line in $output) {
        $text = "$line"
        if ($text -match '^\s*Connected:') { $inConnected = $true; continue }
        if ($text -match '^\s*Persisted:') { $inConnected = $false; continue }
        if (-not $inConnected) { continue }

        # BUSID  VID:PID  DEVICE...  STATE  -- columns separated by 2+ spaces.
        if ($text -match '^\s*(?<busid>\d+-[\d.]+)\s+(?<vidpid>[0-9a-fA-F]{4}:[0-9a-fA-F]{4})\s+(?<rest>.+?)\s*$') {
            if ($Matches.vidpid.ToLower() -ne $EspVidPid) { continue }

            $rest = $Matches.rest
            $busid = $Matches.busid
            $vidpid = $Matches.vidpid.ToLower()
            $parts = $rest -split '\s{2,}'
            $state = $parts[-1].Trim()
            $device = ($parts[0..([Math]::Max(0, $parts.Count - 2))] -join ' ').Trim()

            if ($device -notmatch $EspDescriptionPattern) { continue }

            $hits += [pscustomobject]@{
                BusId  = $busid
                VidPid = $vidpid
                Device = $device
                State  = $state
            }
        }
    }
    return , $hits
}

# Exits non-zero unless exactly one matching device is connected; returns it.
#
# An absent device is polled for a few seconds before giving up: for about a
# second after a detach, Windows re-enumerates the bridge and it is missing from
# `usbipd list`, so a detach-then-attach would otherwise fail spuriously. More
# than one match is never retried — that answer will not improve by waiting.
function Get-SingleEspDevice {
    $deadline = (Get-Date).AddSeconds(5)
    $found = Find-EspDevice
    while ($found.Count -eq 0 -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        $found = Find-EspDevice
    }
    if ($found.Count -eq 0) {
        Write-Host "ERROR: no connected device with VID:PID $EspVidPid (Silicon Labs CP210x USB to UART Bridge) found." -ForegroundColor Red
        Write-Host '       Check the USB cable (it must be a data cable) and run `usbipd list`.'
        exit 1
    }
    if ($found.Count -gt 1) {
        Write-Host "ERROR: $($found.Count) devices match VID:PID $EspVidPid; refusing to guess:" -ForegroundColor Red
        $found | Format-Table BusId, VidPid, Device, State -AutoSize | Out-String | Write-Host
        Write-Host '       Unplug the extra bridge, or run usbipd attach/detach with an explicit --busid.'
        exit 1
    }
    return $found[0]
}
