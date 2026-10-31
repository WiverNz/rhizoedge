# Preferred status: native Windows ps2000. -OwnershipOnly is USB discovery only.
param([switch]$Json, [switch]$OwnershipOnly)
$ErrorActionPreference = 'Stop'
if (-not $OwnershipOnly) {
    $directory=Join-Path (Split-Path $PSScriptRoot -Parent) ('local\evidence\picoscope\status-'+[Guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $directory)
    $request=[ordered]@{action='status';output_dir=$directory;result_file=(Join-Path $directory 'metadata.json');dll=$null;range_v=5;process_timeout_s=20}
    $path=Join-Path $directory 'request.json'
    [IO.File]::WriteAllText($path,($request | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
    $text=& (Join-Path $PSHOME 'powershell.exe') -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'picoscope-windows.ps1') -RequestFile $path
    $code=$LASTEXITCODE
    if ($Json) { $text } else {
        $value=($text -join "`n") | ConvertFrom-Json
        Write-Host "$($value.status) [$($value.backend)]: $($value.message)"
    }
    exit $code
}
. (Join-Path $PSScriptRoot 'picoscope-wsl-common.ps1')
function Finish($state, $code, $message, $device) {
    $r = [ordered]@{ status=$state; exit_code=$code; ready=$false; vid_pid=$PicoVidPid; message=$message; device=$device }
    if ($Json) { $r | ConvertTo-Json -Depth 4 } else { Write-Host "${state}: $message" }
    exit $code
}
if (-not (Get-Command usbipd -ErrorAction SilentlyContinue)) { Finish 'USBIPD MISSING' 10 'Install usbipd-win.' $null }
$list=Invoke-Usbipd list
if ($list.ExitCode -ne 0) { Finish 'USBIPD ERROR' 2 ($list.Output -join ' ') $null }
$found=ConvertFrom-UsbipdList $list.Output
if ($found.Count -eq 0) { Finish 'NOT CONNECTED' 11 'No connected PicoScope 2000 series (0ce9:1007).' $null }
if ($found.Count -gt 1) { Finish 'MULTIPLE DEVICES' 16 'Refusing to guess between matching scopes.' $found }
$dev=$found[0]
if ($dev.State -eq 'Not shared') { Finish 'NOT SHARED' 18 "Run once in Administrator PowerShell: usbipd bind --busid $($dev.BusId)" $dev }
if ($dev.State -match '^Attached') { Finish 'ATTACHED TO WSL' 13 'Windows capture requires detaching with scripts/picoscope-wsl-detach.ps1.' $dev }
if ($dev.State -match '^Shared') { Finish 'WINDOWS AVAILABLE' 19 'Run .\scripts\picoscope-wsl-attach.ps1.' $dev }
Finish 'USBIPD ERROR' 2 "Unknown state: $($dev.State)" $dev
