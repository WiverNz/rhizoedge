<#
.SYNOPSIS
    Report whether the ESP32-C3 development board is connected and ready on
    this Windows host.

.DESCRIPTION
    The first diagnostic to run when unsure whether the real board is
    available. Native Windows only; inside WSL/Linux use ./scripts/esp-status.sh.

    Finds the board's Silicon Labs CP210x / CP2102N bridge (VID:PID 10c4:ea60)
    with `espflash list-ports`, reads the chip with `espflash board-info`, and
    checks `usbipd list` (if usbipd-win is installed) to tell "unplugged" apart
    from "attached to WSL2".

    Non-destructive: it never flashes, erases, binds, attaches, or detaches.
    `espflash board-info` does pulse the reset line, so firmware running on the
    board restarts, exactly as it does when espflash connects for any reason.

    Exit codes:
       0  READY                 board verified, usable from Windows now
       2  UNSUPPORTED           not running on Windows
      10  ESPFLASH MISSING      espflash not on PATH
      11  NOT CONNECTED         no matching bridge visible
      12  PERMISSION ERROR      the port could not be opened for lack of rights
      13  ATTACHED TO WSL       board present but owned by WSL2 (not a fault)
      14  WRONG DEVICE          the bridge answers, but the chip is not an ESP32-C3
      15  BOARD NOT RESPONDING  the port opened, but the chip did not answer
      16  MULTIPLE DEVICES      more than one matching bridge; refuses to guess
      17  PORT BUSY             another program holds the COM port

.PARAMETER Json
    Print one JSON object on stdout instead of the human-readable report.

.EXAMPLE
    .\scripts\esp-status.ps1

.EXAMPLE
    .\scripts\esp-status.ps1 -Json
#>
param([switch]$Json)

$ErrorActionPreference = 'Stop'

$IsWindowsHost = ($PSVersionTable.PSVersion.Major -lt 6) -or $IsWindows

$status = [ordered]@{
    environment       = $null
    ready             = $false
    status            = $null
    exit_code         = $null
    espflash_version  = $null
    serial_port       = $null
    usb_uart          = $null
    vid_pid           = '10c4:ea60'
    usbipd_state      = $null
    usbipd_busid      = $null
    chip              = $null
    revision          = $null
    flash_size        = $null
    crystal_frequency = $null
    wifi              = $null
    ble               = $null
    secure_boot       = $null
    flash_encryption  = $null
    permissions_ok    = $null
    message           = $null
    hint              = $null
}

function Finish([string]$State, [int]$Code, [string]$Message, [string]$Hint) {
    $status.status = $State
    $status.exit_code = $Code
    $status.ready = ($Code -eq 0)
    $status.message = $Message
    $status.hint = if ($Hint) { $Hint } else { $null }

    if ($Json) {
        $status | ConvertTo-Json -Compress:$false
        exit $Code
    }

    $labels = [ordered]@{
        environment       = 'Environment'
        espflash_version  = 'espflash'
        usbipd_state      = 'usbipd'
        serial_port       = 'Serial'
        usb_uart          = 'USB-UART'
        permissions_ok    = 'Permissions'
        chip              = 'Chip'
        crystal_frequency = 'Crystal'
        flash_size        = 'Flash'
        wifi              = 'Wi-Fi/BLE'
        secure_boot       = 'Secure Boot'
        flash_encryption  = 'Flash Encryption'
    }
    Write-Host 'ESP32-C3 status'
    Write-Host '---------------'
    foreach ($key in $labels.Keys) {
        $value = $status[$key]
        if ($null -eq $value) { continue }
        switch ($key) {
            'usbipd_state'   { if ($status.usbipd_busid) { $value = "$value (BUSID $($status.usbipd_busid))" } }
            'permissions_ok' { $value = if ($value) { 'OK' } else { 'DENIED' } }
            'chip'           { if ($status.revision) { $value = "$value rev $($status.revision)" } }
            'wifi'           { $value = if ($status.wifi -and $status.ble) { 'yes' } else { "Wi-Fi $($status.wifi), BLE $($status.ble)" } }
        }
        Write-Host ('{0,-18}{1}' -f "$($labels[$key]):", $value)
    }
    Write-Host ''
    $colour = if ($Code -eq 0) { 'Green' } elseif ($Code -eq 13) { 'Yellow' } else { 'Red' }
    Write-Host "${State}: $Message" -ForegroundColor $colour
    if ($Hint) { Write-Host $Hint }
    exit $Code
}

# Runs an executable with a timeout and returns @{ Output; ExitCode; TimedOut }.
# A plain `& exe` cannot be bounded, and a board-info against a wedged bridge
# can otherwise sit retrying for a long time.
function Invoke-Bounded([string]$Exe, [string]$Arguments, [int]$TimeoutSeconds) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.Arguments = $Arguments
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = $true
    $psi.CreateNoWindow = $true
    $proc = [System.Diagnostics.Process]::Start($psi)
    $proc.StandardInput.Close()
    $out = $proc.StandardOutput.ReadToEndAsync()
    $err = $proc.StandardError.ReadToEndAsync()
    if (-not $proc.WaitForExit($TimeoutSeconds * 1000)) {
        try { $proc.Kill() } catch { }
        return @{ Output = @(); ExitCode = -1; TimedOut = $true }
    }
    $proc.WaitForExit()
    $text = ($out.Result + "`n" + $err.Result) -replace "\x1b\[[0-9;]*m", ''
    $lines = @($text -split "`r?`n" | Where-Object { $_ -ne '' })
    return @{ Output = $lines; ExitCode = $proc.ExitCode; TimedOut = $false }
}

if (-not $IsWindowsHost) {
    Finish 'UNSUPPORTED' 2 'esp-status.ps1 is for native Windows.' 'Inside WSL/Linux run: ./scripts/esp-status.sh'
}

. (Join-Path $PSScriptRoot 'esp-wsl-common.ps1')

$status.environment = "Windows (PowerShell $($PSVersionTable.PSVersion.Major).$($PSVersionTable.PSVersion.Minor))"

# 1. espflash ---------------------------------------------------------------
$espflash = Get-Command espflash -ErrorAction SilentlyContinue
if (-not $espflash) {
    Finish 'ESPFLASH MISSING' 10 'espflash is not on PATH.' 'Install it with: cargo install espflash --locked'
}
$exe = $espflash.Source
$ver = Invoke-Bounded $exe '--version' 15
$status.espflash_version = (($ver.Output | Select-Object -First 1) -replace '^\s*espflash\s+', '').Trim()

# 2. usbipd: who owns the bridge -------------------------------------------
if (Get-Command usbipd -ErrorAction SilentlyContinue) {
    $list = Invoke-Usbipd list
    if ($list.ExitCode -eq 0) {
        $usb = ConvertFrom-UsbipdList $list.Output
        if ($usb.Count -gt 1) {
            Finish 'MULTIPLE DEVICES' 16 "$($usb.Count) CP210x bridges (10c4:ea60) are connected; refusing to guess." "BUSIDs: $(($usb | ForEach-Object { $_.BusId }) -join ', '). Unplug the extra one."
        }
        if ($usb.Count -eq 1) {
            $status.usbipd_state = $usb[0].State
            $status.usbipd_busid = $usb[0].BusId
            $status.usb_uart = $usb[0].Device
        } else {
            $status.usbipd_state = 'not listed'
        }
    } else {
        $status.usbipd_state = 'usbipd list failed'
    }
} else {
    $status.usbipd_state = 'usbipd not installed'
}

if ($status.usbipd_state -match 'Attached') {
    Finish 'ATTACHED TO WSL' 13 'Board detected but currently owned by WSL2.' 'This is not a fault. Use it from WSL (./scripts/esp-status.sh), or return it to Windows with: .\scripts\esp-wsl-detach.ps1'
}

# 3. espflash list-ports ----------------------------------------------------
$ports = Invoke-Bounded $exe 'list-ports --skip-update-check' 30
$hits = @()
foreach ($line in $ports.Output) {
    # espflash prints "PORT  PID:VID  MANUFACTURER  PRODUCT"; accept either order.
    if ($line -match '^\s*(?<port>\S+)\s+(?<ids>[0-9A-Fa-f]{4}:[0-9A-Fa-f]{4})\s*(?<rest>.*)$') {
        $ids = $Matches.ids.ToLower()
        if ($ids -eq '10c4:ea60' -or $ids -eq 'ea60:10c4') {
            $hits += [pscustomobject]@{ Port = $Matches.port; Desc = ($Matches.rest -replace '\s{2,}', ' ').Trim() }
        }
    }
}

if ($hits.Count -gt 1) {
    Finish 'MULTIPLE DEVICES' 16 "$($hits.Count) CP210x serial ports found; refusing to guess." "Ports: $(($hits | ForEach-Object { $_.Port }) -join ', '). Unplug the extra one."
}
if ($hits.Count -eq 0) {
    if ($status.usbipd_state -match 'shared') {
        Finish 'NOT CONNECTED' 11 'usbipd sees the CP210x bridge, but Windows exposes no COM port for it.' 'Check the Silicon Labs CP210x driver in Device Manager, or replug the board.'
    }
    Finish 'NOT CONNECTED' 11 'No CP210x (10c4:ea60) serial port is visible to Windows.' 'Check the USB cable (it must be a data cable) and that the board is powered.'
}

$status.serial_port = $hits[0].Port
# On Windows espflash's description repeats the manufacturer around a NUL byte
# ("Silicon Laboratories\0Silicon Laboratories  Silicon Labs CP210x ..."), so
# usbipd's description is preferred and espflash's is only a cleaned fallback.
if (-not $status.usb_uart -and $hits[0].Desc) {
    $status.usb_uart = ($hits[0].Desc -replace "`0", ' ' -replace '^(Silicon Laborator(y|ies)\s*)+', '' -replace '\s{2,}', ' ').Trim()
}

# 4. espflash board-info ----------------------------------------------------
$info = Invoke-Bounded $exe "board-info --port $($status.serial_port) --non-interactive --skip-update-check" 60
if ($info.TimedOut) {
    Finish 'BOARD NOT RESPONDING' 15 "espflash board-info on $($status.serial_port) did not finish within 60 s." 'Replug the board; if it persists, hold BOOT while pressing RESET and retry.'
}
$text = ($info.Output | Where-Object { $_ -notmatch 'MAC address' }) -join "`n"
if ($info.ExitCode -ne 0) {
    $last = ($info.Output | Where-Object { $_ -notmatch 'MAC address' } | Select-Object -Last 1)
    if ($text -match 'Access is denied|in use|busy') {
        $status.permissions_ok = $true
        Finish 'PORT BUSY' 17 "$($status.serial_port) is held by another program." 'Close any serial monitor (espflash monitor, PuTTY, Arduino IDE) and retry.'
    }
    if ($text -match 'Permission denied') {
        $status.permissions_ok = $false
        Finish 'PERMISSION ERROR' 12 "Cannot open $($status.serial_port): permission denied." ''
    }
    $status.permissions_ok = $true
    Finish 'BOARD NOT RESPONDING' 15 "The port opened but the chip did not answer: $last" 'Replug the board; if it persists, hold BOOT while pressing RESET and retry.'
}
$status.permissions_ok = $true

function Get-Field([string]$Pattern) {
    if ($text -match $Pattern) { return $Matches[1].Trim() }
    return $null
}

$chipRaw = Get-Field '(?m)^\s*Chip type:\s*(\S+)'
$status.revision = Get-Field '(?m)^\s*Chip type:.*\(revision v?([^)]+)\)'
$status.crystal_frequency = Get-Field '(?m)^\s*Crystal frequency:\s*(.+)$'
$flash = Get-Field '(?m)^\s*Flash size:\s*(.+)$'
if ($flash) { $status.flash_size = $flash -replace '(\d)([KMG]B)$', '$1 $2' }
$features = Get-Field '(?m)^\s*Features:\s*(.+)$'
if ($null -ne $features) {
    $status.wifi = $features -match 'WiFi|Wi-Fi'
    $status.ble = $features -match 'BLE'
}
$sb = Get-Field '(?m)^\s*Secure Boot:\s*(.+)$'
if ($sb) { $status.secure_boot = $sb.ToLower() }
$fe = Get-Field '(?m)^\s*Flash Encryption:\s*(.+)$'
if ($fe) { $status.flash_encryption = $fe.ToLower() }

if ($chipRaw) {
    $status.chip = ($chipRaw.ToUpper() -replace '^ESP32(?=[A-Z])', 'ESP32-')
}

if ($chipRaw -ne 'esp32c3') {
    Finish 'WRONG DEVICE' 14 "The CP210x bridge answers, but the chip is '$chipRaw', not an ESP32-C3." 'Another ESP board may be plugged in instead of the Rhizo ESP32-C3.'
}

Finish 'READY' 0 'ESP32-C3 is available for development.' ''
