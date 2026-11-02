# Pure math tests. Does not instantiate Device or load ps2000.dll.
$ErrorActionPreference='Stop'
Add-Type -Path (Join-Path $PSScriptRoot 'PicoScopeWindows.cs')
$count=0
function Assert-True($value,$message) { if (-not $value) { throw $message }; $script:count++ }
Assert-True ([Rhizo.Pico.MathUtil]::RangeIndex(5) -eq 5) '5 V tip must select 500 mV input'
Assert-True ([Rhizo.Pico.MathUtil]::Volts(32767,5) -eq 5) 'Positive scaling'
Assert-True ([Rhizo.Pico.MathUtil]::Volts(-32767,5) -eq -5) 'Negative scaling'
$adc=[Rhizo.Pico.MathUtil]::Threshold(3.3,5)
Assert-True ([Math]::Abs([Rhizo.Pico.MathUtil]::Volts($adc,5)-3.3) -lt 0.0002) 'No double scaling'
$noise=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@(-400,120,0,-100,220),5,0.000001,0.8)
Assert-True ($noise.pulse_count -eq 0) 'Noise must not count as HIGH'
$pulse=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@(0,$adc,$adc,$adc,0,0,$adc,$adc,0),5,0.000001,0.8)
Assert-True ($pulse.pulse_count -eq 2) 'Two pulses'
Assert-True ([Math]::Abs($pulse.pulses[0].width_s-0.000003) -lt 1e-12) 'Three-sample pulse width'
Assert-True ($pulse.pulses[0].start_sample -eq 1 -and $pulse.pulses[0].end_sample -eq 4) 'Transition offsets'
Assert-True ([Math]::Abs($pulse.max_pulse_width_s-0.000003) -lt 1e-12) 'Maximum width'
$censored=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@($adc,$adc,0,$adc),5,0.000001,0.8)
Assert-True ($null -eq $censored.pulses[0].width_s -and $censored.pulses[0].left_censored) 'Left edge unknown'
Assert-True ($null -eq $censored.pulses[1].width_s -and $censored.pulses[1].right_censored) 'Right edge unknown'
$lost=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@(-32768,32767),5,0.000001,0.8)
Assert-True ($lost.lost_data -and $lost.clipped) 'Lost data/clipping invalidates evidence'
$serialized=$pulse | ConvertTo-Json -Depth 8 | ConvertFrom-Json
Assert-True ($serialized.pulses.Count -eq 2 -and $serialized.pulse_count -eq 2) 'Public fields survive JSON'
. (Join-Path $PSScriptRoot 'picoscope-wsl-common.ps1')
$rows=@('Connected:','5-7    0ce9:1007  PicoScope 2000 series PC Oscilloscope  Not shared','Persisted:','5-9    0ce9:1007  PicoScope 2000 series PC Oscilloscope  Attached')
$found=ConvertFrom-UsbipdList $rows
Assert-True ($found.Count -eq 1 -and $found[0].BusId -eq '5-7' -and $found[0].State -eq 'Not shared') 'Discover current BUSID, ignore persisted rows'
$rows=@('Connected:','5-7    0ce9:1007  PicoScope 2000 series PC Oscilloscope  Shared','5-9    0ce9:1007  PicoScope 2000 series PC Oscilloscope  Attached')
$found=ConvertFrom-UsbipdList $rows
Assert-True ($found.Count -eq 2) 'Keep multiple matches for refusal'
$collector=[Rhizo.Pico.StreamCollector]::new(4)
$collector.Append([int16[]]@(1,2),$false,$false)
Assert-True (-not $collector.Complete) 'Partial stream must not pass'
$collector.Append([int16[]]@(3,4),$false,$true)
Assert-True ($collector.Complete -and $collector.samples[2] -eq 3) 'Contiguous callback concatenation'
$collector.Append([int16[]]@(5),$false,$false)
Assert-True (-not $collector.Complete -and $null -ne $collector.error) 'Excess data must not silently truncate'
$shortStream=[Rhizo.Pico.StreamCollector]::new(4)
$shortStream.Append([int16[]]@(1,2),$true,$true)
Assert-True (-not $shortStream.Complete -and $shortStream.overflow) 'Early stop/overflow cannot pass'
$native=[Runtime.InteropServices.Marshal]::AllocHGlobal(8)
try {
 [Runtime.InteropServices.Marshal]::Copy([int16[]]@(11,22,33,44),0,$native,4)
 $direct=[Rhizo.Pico.StreamCollector]::new(4)
 $direct.AppendNative($native,4,$false,$true)
 Assert-True ($direct.Complete -and $direct.samples[3] -eq 44) 'Direct native callback copies exact samples'
 $missing=[Rhizo.Pico.StreamCollector]::new(4)
 $missing.AppendNative([IntPtr]::Zero,4,$false,$true)
 Assert-True (-not $missing.Complete -and $missing.count -eq 0) 'Null native buffer cannot pass'
 $excess=[Rhizo.Pico.StreamCollector]::new(2)
 $excess.AppendNative($native,4,$false,$true)
 Assert-True (-not $excess.Complete -and $excess.count -eq 0) 'Oversized native copy refused before copying'
} finally { [Runtime.InteropServices.Marshal]::FreeHGlobal($native) }
$missingOnly=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@(-32768,-32768),5,0.000001,0.8)
Assert-True ($missingOnly.lost_sample_count -eq 2 -and $null -eq $missingOnly.min_v -and -not $missingOnly.clipped) 'Missing markers are not measured voltages'
$gap=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@(0,$adc,-32768,$adc,0),5,0.000001,0.8)
Assert-True ($gap.lost_sample_count -eq 1 -and $gap.pulse_count -eq 2 -and $null -eq $gap.max_pulse_width_s) 'Never bridge a missing sample or report a precise width across loss'
function Clean-Capture {
 $c=[Rhizo.Pico.Capture]::new();$c.requested_sample_count=3;$c.sample_count=3;$c.sample_interval_s=0.000001;$c.stop_succeeded=$true
 $c.analysis=[Rhizo.Pico.MathUtil]::Analyze([int16[]]@(0,1,0),5,0.000001,0.8)
 return $c
}
$c=Clean-Capture;$c.AssessStream($true,$false)
Assert-True ($c.valid -and $c.contiguous -and -not $c.safety_evidence_valid) 'Clean stream is diagnostic only, never safety-qualified'
$c=Clean-Capture;$c.buffer_overrun=$true;$c.AssessStream($true,$false)
Assert-True (-not $c.safety_evidence_valid -and -not $c.contiguous) 'Overrun invalid even if count matches'
$c=Clean-Capture;$c.analysis=$gap;$c.AssessStream($true,$false)
Assert-True (-not $c.valid -and $c.lost_sample_count -eq 1) 'Missing marker invalid even without overrun flag'
$c=Clean-Capture;$c.sample_count=2;$c.AssessStream($false,$false)
Assert-True (-not $c.valid -and $c.sample_shortfall -eq 1) 'Shortfall invalid, never pad'
$c=Clean-Capture;$c.sample_count=4;$c.AssessStream($true,$false)
Assert-True (-not $c.valid) 'Excess sequence invalid'
$c=Clean-Capture;$c.stop_succeeded=$false;$c.AssessStream($true,$false)
Assert-True (-not $c.valid) 'Stop/finalize failure invalid'
$c=Clean-Capture;$c.overflow=$true;$c.AssessStream($true,$false)
Assert-True (-not $c.valid) 'Driver channel overflow invalid'
$c=Clean-Capture;$c.collector_error='unexpected callback';$c.AssessStream($true,$false)
Assert-True (-not $c.valid) 'Callback lifecycle error invalid'
$c=Clean-Capture;$c.AssessStream($true,$true)
Assert-True (-not $c.valid -and $c.exit_code -eq 22) 'Timeout invalid even with complete array'
$parts=[Rhizo.Pico.StreamCollector]::new(6)
$parts.Append([int16[]]@(11,22),$false,$false);$parts.Append([int16[]]@(),$false,$false);$parts.Append([int16[]]@(33,44,55,66),$false,$true)
Assert-True ($parts.Complete -and ($parts.samples -join ',') -eq '11,22,33,44,55,66') 'Variable and empty callbacks preserve exact ordering'
$c=Clean-Capture;$c.sample_interval_s=0.0000005;$c.AssessStream($true,$false)
Assert-True (-not $c.safety_evidence_valid) 'Out-of-spec streaming rate cannot qualify even with clean data'
function Clean-Block {
 $c=Clean-Capture;$c.sampling_mode='block';$c.block_ready=$true;$c.timestamps_sample_count=3;$c.timestamp_sequence_valid=$true;$c.readbacks_equal=$true
 return $c
}
$c=Clean-Block;$c.AssessBlock($false)
Assert-True ($c.valid -and $c.safety_evidence_valid) 'Complete bounded block is eligible evidence, not HIL pass'
$c=Clean-Block;$c.sample_count=2;$c.AssessBlock($false)
Assert-True (-not $c.valid -and $c.sample_shortfall -eq 1) 'Short block read fails closed'
$c=Clean-Block;$c.timestamps_sample_count=2;$c.AssessBlock($false)
Assert-True (-not $c.valid) 'Incomplete timestamp read fails closed'
$c=Clean-Block;$c.timestamp_sequence_valid=$false;$c.AssessBlock($false)
Assert-True (-not $c.valid) 'Wrong driver interval invalid'
$c=Clean-Block;$c.readbacks_equal=$false;$c.AssessBlock($false)
Assert-True (-not $c.valid) 'Independent readback disagreement invalid'
$c=Clean-Block;$c.AssessBlock($true)
Assert-True (-not $c.safety_evidence_valid -and $c.exit_code -eq 22) 'No trigger timeout is not absence evidence'
$c=Clean-Block;$c.overflow=$true;$c.AssessBlock($false)
Assert-True (-not $c.valid) 'Block overflow invalid'
$c=Clean-Block;$c.analysis=$gap;$c.AssessBlock($false)
Assert-True (-not $c.valid -and $c.lost_sample_count -eq 1) 'Block loss invalid'
$c=Clean-Block;$c.stop_succeeded=$false;$c.AssessBlock($false)
Assert-True (-not $c.valid) 'Failed stop invalid'
Assert-True ([Rhizo.Pico.MathUtil]::TimestampsValid([int[]]@(-80,1,83,165,247,329,411,492),0.00008192,0.000001)) 'Signed integer timestamp quantization permits two endpoint units'
Assert-True (-not [Rhizo.Pico.MathUtil]::TimestampsValid([int[]]@(0,640,1920),0.000000640,0.000000001)) 'Skipped timestamp sample invalid'
Assert-True (-not [Rhizo.Pico.MathUtil]::TimestampsValid([int[]]@(0,640,640),0.000000640,0.000000001)) 'Repeated timestamp sample invalid'
Assert-True (-not [Rhizo.Pico.MathUtil]::TimestampsValid([int[]]@(0,1280,2560),0.000000640,0.000000001)) 'Wrong scale invalid'
$temp=Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
[IO.Directory]::CreateDirectory($temp) | Out-Null
try {
 [Rhizo.Pico.EvidenceSignal]::Publish($temp,1,'first')
 $held=[IO.FileStream]::new((Join-Path $temp 'armed.txt'),[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
 try {
  for($i=2;$i -le 100;$i++) { [Rhizo.Pico.EvidenceSignal]::Publish($temp,$i,'complete') }
  Assert-True ([IO.File]::ReadAllText((Join-Path $temp 'armed.txt')) -eq '100 complete') 'Atomic arm notifications survive a held supervisor read handle'
  $reader=[IO.StreamReader]::new($held)
  try { Assert-True ($reader.ReadToEnd() -eq '1 first') 'Reader sees complete original message, never torn content' } finally { $reader.Dispose() }
 } finally { $held.Dispose() }
} finally { try { [IO.Directory]::Delete($temp,$true) } catch { Write-Warning ('Temporary cleanup failed: '+$_.Exception.Message) } }
Write-Output "$count Windows math/JSON/discovery assertions passed; no hardware API calls."
