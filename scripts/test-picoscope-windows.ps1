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
Write-Output "$count Windows math/JSON/discovery assertions passed; no hardware API calls."
