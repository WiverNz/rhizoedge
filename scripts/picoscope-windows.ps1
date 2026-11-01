# Native Windows ps2000 backend. Supervisor never opens USB; worker owns mutex.
param([Parameter(Mandatory=$true)][string]$RequestFile, [switch]$Worker)
$ErrorActionPreference='Stop'
$r=Get-Content -Raw -LiteralPath $RequestFile | ConvertFrom-Json
$utf8=New-Object System.Text.UTF8Encoding($false)
function Save-Result($value) {
    [IO.File]::WriteAllText($r.result_file,($value | ConvertTo-Json -Depth 12)+"`n",$utf8)
}
if (-not $Worker) {
    # Files are unique per invocation; no user-supplied command text is evaluated.
    if ($RequestFile.Contains('"') -or $PSCommandPath.Contains('"')) { throw 'Quote in script/request path' }
    $psi=New-Object Diagnostics.ProcessStartInfo
    $psi.FileName=Join-Path $PSHOME 'powershell.exe'
    $psi.Arguments='-NoProfile -ExecutionPolicy Bypass -File "'+$PSCommandPath+'" -RequestFile "'+$RequestFile+'" -Worker'
    $psi.UseShellExecute=$false; $psi.CreateNoWindow=$true
    $psi.RedirectStandardOutput=$true; $psi.RedirectStandardError=$true
    $p=[Diagnostics.Process]::Start($psi)
    $out=$p.StandardOutput.ReadToEndAsync(); $err=$p.StandardError.ReadToEndAsync()
    $watch=[Diagnostics.Stopwatch]::StartNew(); $lastArm=''
    while (-not $p.WaitForExit(100)) {
        $arm=Join-Path $r.output_dir 'armed.txt'
        if (Test-Path -LiteralPath $arm) {
            $current=[IO.File]::ReadAllText($arm)
            if ($current -ne $lastArm) { [Console]::Error.WriteLine('ARMED '+$current); $lastArm=$current }
        }
        if ($watch.Elapsed.TotalSeconds -ge $r.process_timeout_s) {
            try { $p.Kill() } catch { }
            $exited=$p.WaitForExit(2000)
            Save-Result ([ordered]@{backend='windows-ps2000';status='BACKEND TIMEOUT';ready=$false;exit_code=21;worker_pid=$p.Id;worker_exited=$exited;message='Worker exceeded total deadline. No automatic retry. Partial files are not valid evidence.'})
            Get-Content -Raw -LiteralPath $r.result_file
            exit 21
        }
    }
    if (-not (Test-Path -LiteralPath $r.result_file)) {
        Save-Result ([ordered]@{backend='windows-ps2000';status='BACKEND FAILED';ready=$false;exit_code=21;message='Worker returned no result.';diagnostic=$err.Result})
    }
    $result=Get-Content -Raw -LiteralPath $r.result_file
    Write-Output $result
    exit ($result | ConvertFrom-Json).exit_code
}
$result=[ordered]@{schema_version=1;backend='windows-ps2000';status='DEVICE OPEN FAILED';ready=$false;exit_code=21;captured=$false}
$device=$null
try {
    . (Join-Path $PSScriptRoot 'picoscope-wsl-common.ps1')
    if (-not (Get-Command usbipd -ErrorAction SilentlyContinue)) { $result.status='USBIPD MISSING';$result.exit_code=10;throw 'usbipd ownership discovery unavailable' }
    $list=Invoke-Usbipd list
    if ($list.ExitCode -ne 0) { throw 'usbipd list failed' }
    $found=ConvertFrom-UsbipdList $list.Output
    if ($found.Count -eq 0) { $result.status='NOT CONNECTED';$result.exit_code=11;throw 'No connected PicoScope 0ce9:1007' }
    if ($found.Count -gt 1) { $result.status='MULTIPLE DEVICES';$result.exit_code=16;throw 'Refusing to guess between scopes' }
    $result.usb_state=$found[0].State
    if ($found[0].State -match '^Attached') { $result.status='ATTACHED TO WSL';$result.exit_code=13;throw 'Detach with scripts/picoscope-wsl-detach.ps1 before Windows capture' }
    if ($found[0].State -notmatch '^(Shared|Not shared)$') { throw 'Unknown USB ownership state' }
    # Not shared is fine: Windows operation requires neither binding nor attach.
    $dll=$r.dll
    if (-not $dll) { $dll=Join-Path $env:ProgramFiles 'Pico Technology\PicoScope 7 T&M Stable\ps2000.dll' }
    if (-not (Test-Path -LiteralPath $dll)) { $result.status='DRIVER/API MISSING';$result.exit_code=20;throw 'ps2000.dll missing; specify --dll with the installed x64 SDK path' }
    Add-Type -Path (Join-Path $PSScriptRoot 'PicoScopeWindows.cs')
    $device=[Rhizo.Pico.Device]::new([string]$dll,[double]$r.range_v)
    $result.model=$device.model;$result.driver_version=$device.driver_version;$result.serial_queried=$device.serial_queried
    $result.channel='A';$result.coupling='DC';$result.probe_attenuation=10;$result.range_v=$r.range_v;$result.input_range_v=$r.range_v/10
    $result.requested=$r
    if ($r.action -eq 'capture') {
        if ($r.test_signal) {
            $device.TestSignal([string]$r.test_signal)
            $result.test_signal=[ordered]@{kind=$r.test_signal;isolated_loop_only=$true;cleanup='DC zero before normal close; forced termination cannot guarantee cleanup'}
        }
        $records=New-Object 'Collections.Generic.List[object]'
        for($i=1;$i -le $r.count;$i++) {
            if ($r.mode -eq 'stream') {
                $capture=$device.Stream([double]$r.duration_s,[uint32]$r.stream_interval_ns,[double]$r.range_v,[double]$r.pulse_v,[double]$r.timeout_s,[string]$r.output_dir,($r.high_resolution_timer -ne $false),[bool]$r.diagnostic_readback)
            } else {
            $capture=$device.Acquire($i,[double]$r.duration_s,[int]$r.samples,[double]$r.range_v,[string]$r.trigger,[double]$r.trigger_v,[double]$r.pulse_v,[double]$r.pre_s,[double]$r.timeout_s,[string]$r.output_dir)
            }
            $records.Add($capture)
            [IO.File]::WriteAllText((Join-Path $r.output_dir ('capture-{0:D4}.json' -f $i)),($capture | ConvertTo-Json -Depth 10)+"`n",$utf8)
            if ($capture.exit_code -ne 0) { break }
        }
        $result.captures=@($records.ToArray())
        $result.completed_count=$records.Count
        $result.exit_code=$records[$records.Count-1].exit_code
        $result.status=$records[$records.Count-1].status
        $result.captured=$result.exit_code -eq 0 -and $records.Count -eq $r.count
        $result.measurement_validation='UNVALIDATED: requires real LOW, HIGH, trigger and width checks before HIL'
        $result.inter_capture_gaps=if ($r.mode -eq 'stream') {'Single stream; contiguous is true only with complete data and no detected overrun/loss.'} else {'Block acquisitions have dead time; count does not imply continuous coverage.'}
    } elseif ($r.action -eq 'status') {
        $result.status='READY';$result.exit_code=0
    } else { throw 'Unknown action' }
    # READY/success is emitted only after successful close.
    $device.Dispose();$device=$null
    $result.ready=$result.exit_code -eq 0
} catch {
    $result.message=$_.Exception.Message
    $result.ready=$false;$result.captured=$false
    if ($result.exit_code -eq 0) { $result.exit_code=21;$result.status='DEVICE OPEN FAILED' }
} finally {
    if ($null -ne $device) {
        try { $device.Dispose() } catch { $result.message=$_.Exception.Message;$result.status='DEVICE CLOSE FAILED';$result.exit_code=21;$result.ready=$false;$result.captured=$false }
    }
    Save-Result $result
}
exit $result.exit_code
