// ps2000 ABI: vendor ps2000.h / ps2000pg.en-12. Windows x64 only.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace Rhizo.Pico {
 public static class EvidenceSignal {
  public static void Publish(string directory,int acquisition,string timestamp) {
   string target=Path.Combine(directory,"armed.txt"),pending=Path.Combine(directory,"armed.next");
   File.WriteAllText(pending,acquisition+" "+timestamp);
   if(File.Exists(target))File.Replace(pending,target,null);else File.Move(pending,target);
  }
 }
 public sealed class BlockLimit {
  public int enabled_channels,timebase,interval_ns,max_samples,units_hint;
  public short return_code,full_count_return_code;
  public int full_count_interval_ns,full_count_max_samples;
 }
 public sealed class Pulse {
  public int start_sample, end_sample;
  public double start_offset_s, end_offset_s, observed_width_s;
  public double? width_s;
  public bool left_censored, right_censored;
 }
 public sealed class Analysis {
  public double? min_v, max_v, peak_to_peak_v, max_pulse_width_s;
  public int pulse_count;
  public List<Pulse> pulses = new List<Pulse>();
  public bool clipped, lost_data;
  public int lost_sample_count, valid_sample_count;
  public int? first_lost_sample, last_lost_sample;
 }
 public sealed class StreamCollector {
  public readonly short[] samples;
  public int count; public bool overflow, autoStopped; public string error;
  public StreamCollector(int size) { samples=new short[size]; }
  public void Append(short[] chunk,bool clipped,bool stopped) {
   overflow |= clipped; autoStopped |= stopped;
   if(chunk.Length>samples.Length-count) { error="Streaming exceeded requested sample count";return; }
   Array.Copy(chunk,0,samples,count,chunk.Length);count+=chunk.Length;
  }
  public void AppendNative(IntPtr buffer,int length,bool clipped,bool stopped) {
   overflow |= clipped; autoStopped |= stopped;
   if(length<0 || length>samples.Length-count) {error="Streaming exceeded requested sample count";return;}
   if(length>0 && buffer==IntPtr.Zero) {error="Missing channel A buffer";return;}
   if(length>0)Marshal.Copy(buffer,samples,count,length);
   count+=length;
  }
  public bool Complete { get {return error==null && autoStopped && count==samples.Length;} }
 }
 public sealed class Capture {
  public int acquisition, sample_count, requested_sample_target, timebase, exit_code;
  public string sampling_mode="block", raw_i16, raw_format, interval_source, started_utc;
  public bool buffer_overrun, auto_stopped, contiguous;
  public double stream_wall_s, max_poll_gap_s;
  public double? first_overrun_wall_s;
  public int overview_buffer_samples=1000000, callback_count;
  public double requested_duration_s, sample_interval_s, sample_rate_hz, duration_s, sample_span_s;
  public double requested_pretrigger_s, nominal_pretrigger_samples;
  public double range_v, input_range_v, pulse_threshold_v;
  public double? trigger_threshold_v, trigger_threshold_actual_v;
  public short? trigger_threshold_adc;
  public bool? triggered;
  public bool overflow, valid;
  public string trigger_direction, armed_utc, completed_utc, raw_csv, status;
  public Analysis analysis;
  public int requested_sample_count, lost_sample_count, sample_shortfall, callback_samples_min, callback_samples_max;
  public bool timer_resolution_released, diagnostic_readback;
  public int timer_resolution_requested_ms, readback_sample_count, readback_lost_count, readback_mismatch_count;
  public double readback_s;
  public int polling_count, empty_poll_count, gc0_collections, gc1_collections, gc2_collections;
  public short driver_overflow_flags;
  public double max_callback_gap_s, max_copy_s, total_copy_s, max_poll_call_s, max_status_call_s, stop_s, analysis_s, writer_s;
  public bool stop_succeeded, safety_evidence_valid;
  public bool block_ready, timestamp_sequence_valid, readbacks_equal;
  public int block_capacity_samples, timestamps_sample_count, timestamp_unit_code;
  public double timestamp_quantum_s;
  public string channels="A", trigger_source="A", selection_mode, raw_b_csv, safety_evidence_policy;
  public Analysis analysis_b;
  public string collector_error, continuity_basis="ps2000 count/auto-stop/overrun/sentinel checks; no device sequence numbers or per-sample timestamps exposed";
  public List<string> invalid_reasons=new List<string>();
  public void AssessBlock(bool timedOut) {
   invalid_reasons.Clear();lost_sample_count=analysis==null?0:analysis.lost_sample_count;
   if(analysis_b!=null)lost_sample_count+=analysis_b.lost_sample_count;
   sample_shortfall=Math.Max(0,requested_sample_count-sample_count);
   if(timedOut || !block_ready)invalid_reasons.Add("block did not complete; no absence evidence");
   if(sample_count!=requested_sample_count || timestamps_sample_count!=requested_sample_count)invalid_reasons.Add("incomplete block readback");
   if(!timestamp_sequence_valid)invalid_reasons.Add("driver timestamps disagree with selected interval");
   if(!readbacks_equal)invalid_reasons.Add("get_values and get_times_and_values disagree");
   if(overflow)invalid_reasons.Add("enabled channel ADC overflow");
   if(analysis==null || lost_sample_count>0)invalid_reasons.Add("missing block data");
   if((analysis!=null && analysis.clipped) || (analysis_b!=null && analysis_b.clipped))invalid_reasons.Add("ADC clipping");
   if(collector_error!=null)invalid_reasons.Add(collector_error);
   if(!stop_succeeded)invalid_reasons.Add("stop failed");
   valid=invalid_reasons.Count==0;contiguous=valid;
   safety_evidence_valid=valid;
   safety_evidence_policy="Bounded block only; event/reference coverage and pulse sensitivity must be established separately; never implies HIL PASS";
   exit_code=timedOut?22:valid?0:23;status=valid?"CAPTURED":"INVALID CAPTURE";
  }
  public void AssessStream(bool complete,bool timedOut) {
   lost_sample_count=analysis==null?0:analysis.lost_sample_count;
   sample_shortfall=Math.Max(0,requested_sample_count-sample_count);
   invalid_reasons.Clear();
   if(sample_interval_s<1e-6)invalid_reasons.Add("Above the published 2205A 1 MS/s streaming limit");
   if(timedOut)invalid_reasons.Add("capture timeout");
   if(!complete || sample_count!=requested_sample_count)invalid_reasons.Add("incomplete or excess sample sequence / missing auto-stop");
   if(collector_error!=null)invalid_reasons.Add(collector_error);
   if(buffer_overrun)invalid_reasons.Add("driver overview buffer overrun");
   if(lost_sample_count>0)invalid_reasons.Add("driver missing-sample markers");
   if(analysis==null)invalid_reasons.Add("no samples");
   if(overflow)invalid_reasons.Add("driver channel overflow");
   if(analysis!=null && analysis.clipped)invalid_reasons.Add("ADC clipping");
   if(!stop_succeeded)invalid_reasons.Add("stop failed / finalization not synchronized");
   contiguous=complete && sample_count==requested_sample_count && collector_error==null && !buffer_overrun && analysis!=null && lost_sample_count==0 && stop_succeeded;
   valid=invalid_reasons.Count==0;
   safety_evidence_valid=false;
   safety_evidence_policy="Streaming is NOT QUALIFIED for M9 safety evidence on this bench; diagnostics only";
   exit_code=timedOut?22:valid?0:23;
   status=valid?"CAPTURED":"INVALID CAPTURE";
  }
 }
 public static class MathUtil {
  public static readonly double[] InputRanges = { .01,.02,.05,.1,.2,.5,1,2,5,10,20,50 };
  public static int RangeIndex(double tipRange) {
   for (int i=1;i<=10;i++) if (Math.Abs(InputRanges[i]*10-tipRange)<1e-9) return i;
   throw new ArgumentException("Unsupported probe-tip range; x10 is fixed.");
  }
  public static double Volts(short adc, double tipRange) { return adc * tipRange / 32767.0; }
  public static short Threshold(double volts, double tipRange) {
   if (Double.IsNaN(volts) || Double.IsInfinity(volts) || Math.Abs(volts)>=tipRange) throw new ArgumentException("Threshold must be inside probe-tip range.");
   return checked((short)Math.Round(volts / tipRange * 32767.0));
  }
  public static bool TimestampsValid(int[] times,double interval,double quantum) {
   if(times==null || times.Length<2 || interval<=0 || quantum<=0)return false;
   // Each integer timestamp can lose up to one time-unit on conversion.
   // Subtracting two independently quantized timestamps permits two units,
   // including a negative pretrigger origin and positive later samples.
   for(int i=1;i<times.Length;i++)
    if(times[i]<=times[i-1] || Math.Abs(((long)times[i]-times[0])*quantum-i*interval)>quantum*2.01)return false;
   return true;
  }
  public static Analysis Analyze(short[] samples, double tipRange, double interval, double threshold) {
   if (samples.Length==0 || interval<=0) throw new ArgumentException("Nonempty samples and positive interval required.");
   Analysis a=new Analysis(); double min=Double.PositiveInfinity,max=Double.NegativeInfinity;
   int start=-1;
   for(int i=0;i<samples.Length;i++) {
    if(samples[i]==-32768) {
     a.lost_data=true;a.lost_sample_count++;if(a.first_lost_sample==null)a.first_lost_sample=i;a.last_lost_sample=i;
     if(start>=0){AddPulse(a,start,i,samples.Length,interval,true);start=-1;}
     continue;
    }
    a.valid_sample_count++;
    double v=Volts(samples[i],tipRange); min=Math.Min(min,v); max=Math.Max(max,v);
    a.clipped |= Math.Abs((int)samples[i])>=32767;
    if(v>threshold && start<0) start=i;
    if(v<=threshold && start>=0) { AddPulse(a,start,i,samples.Length,interval,false); start=-1; }
   }
   if(start>=0) AddPulse(a,start,samples.Length-1,samples.Length,interval,true);
   if(a.valid_sample_count>0){a.min_v=min;a.max_v=max;a.peak_to_peak_v=max-min;}
   if(a.lost_data){foreach(Pulse p in a.pulses)p.width_s=null;a.max_pulse_width_s=null;}
   a.pulse_count=a.pulses.Count;
   return a;
  }
  static void AddPulse(Analysis a,int start,int end,int count,double dt,bool right) {
   Pulse p=new Pulse { start_sample=start,end_sample=end,start_offset_s=start*dt,end_offset_s=end*dt,
    observed_width_s=(end-start)*dt,left_censored=start==0,right_censored=right };
   if(!p.left_censored && !p.right_censored) { p.width_s=p.observed_width_s; a.max_pulse_width_s=Math.Max(a.max_pulse_width_s??0,p.width_s.Value); }
   a.pulses.Add(p);
  }
 }
 public sealed class Device : IDisposable {
  [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint period);
  [DllImport("winmm.dll")] static extern uint timeEndPeriod(uint period);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate uint StreamReadback(short h,ref double start,[Out] short[] a,IntPtr b,IntPtr c,IntPtr d,out short overflow,out uint triggerAt,out short triggered,uint count);
  [DllImport("kernel32",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr LoadLibraryEx(string path,IntPtr reserved,uint flags);
  [DllImport("kernel32",CharSet=CharSet.Ansi,ExactSpelling=true)] static extern IntPtr GetProcAddress(IntPtr module,string name);
  [DllImport("kernel32")] static extern bool FreeLibrary(IntPtr module);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Open();
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Handle(short h);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Info(short h,StringBuilder text,short size,short line);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Channel(short h,short ch,short enabled,short dc,short range);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Timebase(short h,short tb,int n,out int interval,out short units,short oversample,out int max);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Trigger(short h,short source,short threshold,short direction,float delay,short autoMs);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Run(short h,int n,short tb,short oversample,out int ms);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int Values(short h,[Out] short[] a,[Out] short[] b,IntPtr c,IntPtr d,out short overflow,int n);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int TimesValues(short h,[Out] int[] times,[Out] short[] a,[Out] short[] b,IntPtr c,IntPtr d,out short overflow,short units,int n);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Signal(short h,int offset,uint peakToPeak,int wave,float start,float end,float increment,float dwell,int sweep,uint sweeps);
  Signal signal; bool generatorConfigured;
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short StreamRun(short h,uint interval,int units,uint max,short autoStop,uint aggregate,uint overview);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate void StreamCallback(IntPtr buffers,short overflow,uint triggeredAt,short triggered,short autoStop,uint count);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short StreamPoll(short h,StreamCallback callback);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short StreamStatus(short h,out short overrun);
  IntPtr module; short handle; Mutex mutex; bool owns;
  Open open; Handle close,stop,ready; Info info; Channel channel; Timebase timebase; Trigger trigger; Run run; Values values;
  public string model,driver_version; public bool serial_queried;
  T Bind<T>(string name) where T:class {
   IntPtr address=GetProcAddress(module,"ps2000_"+name);
   if(address==IntPtr.Zero) throw new EntryPointNotFoundException(name);
   return Marshal.GetDelegateForFunctionPointer(address,typeof(T)) as T;
  }
  public Device(string dll, double tipRange) {
   if(!Environment.Is64BitProcess) throw new InvalidOperationException("Windows x64 process required.");
   mutex=new Mutex(false,"Local\\RhizoEdge.PicoScope.ps2000");
   try {
    try { owns=mutex.WaitOne(0); } catch(AbandonedMutexException) { owns=true; }
    if(!owns) throw new InvalidOperationException("Another Windows ps2000 worker holds the device mutex.");
    module=LoadLibraryEx(Path.GetFullPath(dll),IntPtr.Zero,0x1100);
    if(module==IntPtr.Zero) throw new DllNotFoundException("ps2000 DLL load failed: "+Marshal.GetLastWin32Error());
    open=Bind<Open>("open_unit"); close=Bind<Handle>("close_unit"); stop=Bind<Handle>("stop"); ready=Bind<Handle>("ready");
    info=Bind<Info>("get_unit_info"); channel=Bind<Channel>("set_channel"); timebase=Bind<Timebase>("get_timebase");
    trigger=Bind<Trigger>("set_trigger2"); run=Bind<Run>("run_block"); values=Bind<Values>("get_values");
    handle=open(); if(handle<=0) throw new InvalidOperationException("ps2000_open_unit returned "+handle);
    model=GetInfo(3); if(model!="2205A") throw new InvalidOperationException("Expected 2205A, got "+model);
    driver_version=GetInfo(0); serial_queried=GetInfo(4).Length>0;
    Check(channel(handle,0,1,1,(short)MathUtil.RangeIndex(tipRange)),"Channel A DC");
    Check(channel(handle,1,0,1,5),"Channel B disable");
   } catch { Dispose(); throw; }
  }
  string GetInfo(short index) { StringBuilder b=new StringBuilder(256); Check(info(handle,b,256,index),"get_unit_info"); return b.ToString(); }
  static void Check(short rc,string op) { if(rc<=0) throw new InvalidOperationException(op+" failed: "+rc); }
  // Explicit isolated probe-loop check only. Normal status/capture never enables output.
  public void TestSignal(string kind) {
   if(kind!="square-1khz" && kind!="dc-zero") throw new ArgumentException("Unknown test signal");
   signal=Bind<Signal>("set_sig_gen_built_in"); generatorConfigured=true;
   if(kind=="square-1khz") Check(signal(handle,1000000,2000000,1,1000,1000,0,1,0,0),"test signal 0..2 V, 1 kHz");
   else Check(signal(handle,0,0,5,1,1,0,1,0,0),"test signal DC zero");
  }
  public List<BlockLimit> ProbeBlockLimits() {
   List<BlockLimit> limits=new List<BlockLimit>();
   try {
    for(short enabled=0;enabled<=1;enabled++) {
     Check(channel(handle,1,enabled,1,5),"Channel B for block capability query");
     for(short tb=0;tb<=23;tb++) {
      int dt,max;short units;short rc=timebase(handle,tb,2,out dt,out units,1,out max);
      BlockLimit row=new BlockLimit {enabled_channels=enabled+1,timebase=tb,interval_ns=dt,max_samples=max,units_hint=units,return_code=rc};
      if(rc>0 && max>=2) {
       int fullDt,fullMax;short fullUnits;
       row.full_count_return_code=timebase(handle,tb,max,out fullDt,out fullUnits,1,out fullMax);
       row.full_count_interval_ns=fullDt;row.full_count_max_samples=fullMax;
      }
      limits.Add(row);
     }
    }
   } finally {Check(channel(handle,1,0,1,5),"Restore Channel B off");}
   return limits;
  }
  public Capture Acquire(int acquisition,double duration,int target,double tipRange,string direction,double threshold,double pulseThreshold,double pre,double timeout,string directory,int fixedTimebase=-1,int fixedCount=0,bool enableB=false,string triggerSource="A") {
   if(duration<=0 || target<2 || target>1000000 || pre<0 || timeout<=0 || fixedTimebase < -1 || fixedTimebase>23)
    throw new ArgumentException("Invalid block settings");
   if((fixedTimebase<0)!=(fixedCount==0) || (fixedCount!=0 && (fixedCount<2 || fixedCount>1000000)))throw new ArgumentException("Fixed timebase/count must be supplied together");
   if(triggerSource!="A" && triggerSource!="B")throw new ArgumentException("Invalid trigger source");
   if(triggerSource=="B" && !enableB)throw new ArgumentException("B trigger requires B enabled");
   MathUtil.Threshold(pulseThreshold,tipRange);
   Check(channel(handle,1,enableB?(short)1:(short)0,1,(short)MathUtil.RangeIndex(tipRange)),"Channel B block configuration");
   short selected=-1;int interval=0,n=0,capacity=0;
   if(fixedTimebase>=0) {
    int dt,max;short units;
    Check(timebase(handle,(short)fixedTimebase,fixedCount,out dt,out units,1,out max),"fixed get_timebase");
    if(dt<=0 || fixedCount>max)throw new InvalidOperationException("Fixed block exceeds real device capacity; refusing slower timebase fallback");
    selected=(short)fixedTimebase;interval=dt;n=fixedCount;capacity=max;duration=n*dt*1e-9;
   } else {
    for(short tb=0;tb<=23;tb++) {
     int dt,max;short units;
     if(timebase(handle,tb,2,out dt,out units,1,out max)<=0 || dt<=0)continue;
     double needed=Math.Ceiling(duration/(dt*1e-9));
     if(needed>=2 && needed<=Math.Min(target,max)){selected=tb;interval=dt;n=(int)needed;capacity=max;break;}
    }
   }
   if(selected<0)throw new InvalidOperationException("No timebase meets duration/sample target");
   double seconds=interval*1e-9;
   if(pre>=n*seconds || timeout<=n*seconds)throw new ArgumentException("Require pretrigger < block duration < timeout");
   bool triggeredMode=direction!="none";float delay=triggeredMode?(float)(-100*pre/(n*seconds)):0;
   short source=triggeredMode?(triggerSource=="B"?(short)1:(short)0):(short)5;
   Check(trigger(handle,source,triggeredMode?MathUtil.Threshold(threshold,tipRange):(short)0,direction=="falling"?(short)1:(short)0,delay,0),"set_trigger2");
   Capture c=new Capture {acquisition=acquisition,requested_duration_s=duration,requested_sample_target=target,requested_sample_count=n,
    block_capacity_samples=capacity,selection_mode=fixedTimebase>=0?"fixed timebase and count; no fallback":"automatic duration fit; inspect actual rate",channels=enableB?"AB":"A",trigger_source=triggeredMode?triggerSource:"none",
    timebase=selected,sample_interval_s=seconds,sample_rate_hz=1/seconds,range_v=tipRange,input_range_v=tipRange/10,
    interval_source="ps2000_get_timebase nanoseconds; verified against get_times_and_values",
    trigger_direction=direction,trigger_threshold_v=triggeredMode?(double?)threshold:null,pulse_threshold_v=pulseThreshold,
    trigger_threshold_adc=triggeredMode?(short?)MathUtil.Threshold(threshold,tipRange):null,
    trigger_threshold_actual_v=triggeredMode?(double?)MathUtil.Volts(MathUtil.Threshold(threshold,tipRange),tipRange):null,
    requested_pretrigger_s=pre,nominal_pretrigger_samples=triggeredMode?-delay*n/100.0:0,
    continuity_basis="Internal block complete before transfer; exact counts, driver timestamp spacing, two readbacks, overflow/loss flags; gaps between blocks are not covered"};
   TimesValues getTimes=Bind<TimesValues>("get_times_and_values");
   int ms;Check(run(handle,n,selected,1,out ms),"run_block");
   c.started_utc=c.armed_utc=DateTime.UtcNow.ToString("o");
   Stopwatch clock=Stopwatch.StartNew();bool timedOut=false;
   short[] raw=null,rawB=null;int[] times=null;
   try {
    EvidenceSignal.Publish(directory,acquisition,c.armed_utc);
    while(true) {
     short r=ready(handle);if(r<0)throw new InvalidOperationException("ps2000_ready failed");if(r>0){c.block_ready=true;break;}
     if(clock.Elapsed.TotalSeconds>=timeout){timedOut=true;break;}Thread.Sleep(2);
    }
    if(c.block_ready) {
     raw=new short[n];rawB=enableB?new short[n]:null;short overflow;
     int got=values(handle,raw,rawB,IntPtr.Zero,IntPtr.Zero,out overflow,n);
     if(got<0 || got>n)throw new InvalidOperationException("Invalid get_values count");
     c.sample_count=got;c.driver_overflow_flags=overflow;
     short[] timedRaw=new short[n],timedB=enableB?new short[n]:null;times=new int[n];short timedOverflow;
     short unit=2;double quantum=1e-9;
     while(n*seconds/quantum>int.MaxValue/1.01 && unit<5){unit++;quantum*=1000;}
     c.timestamp_unit_code=unit;c.timestamp_quantum_s=quantum;
     int timedGot=getTimes(handle,times,timedRaw,timedB,IntPtr.Zero,IntPtr.Zero,out timedOverflow,unit,n);
     if(timedGot<0 || timedGot>n)throw new InvalidOperationException("Invalid get_times_and_values count");
     c.timestamps_sample_count=timedGot;c.driver_overflow_flags|=timedOverflow;
     c.readbacks_equal=got==timedGot;
     for(int i=0;i<Math.Min(got,timedGot);i++)if(raw[i]!=timedRaw[i] || (enableB && rawB[i]!=timedB[i]))c.readbacks_equal=false;
     int[] observedTimes=new int[timedGot];Array.Copy(times,observedTimes,timedGot);
     c.timestamp_sequence_valid=MathUtil.TimestampsValid(observedTimes,seconds,quantum);
     c.overflow=(c.driver_overflow_flags&(enableB?3:1))!=0;
     Array.Resize(ref raw,got);if(enableB)Array.Resize(ref rawB,got);Array.Resize(ref times,timedGot);
     c.duration_s=got*seconds;c.sample_span_s=Math.Max(0,got-1)*seconds;
     if(got>0){c.analysis=MathUtil.Analyze(raw,tipRange,seconds,pulseThreshold);if(enableB)c.analysis_b=MathUtil.Analyze(rawB,tipRange,seconds,pulseThreshold);}
    }
   } catch(Exception e){c.collector_error=e.Message;}
   finally {c.stop_succeeded=stop(handle)>0;}
   c.triggered=triggeredMode?(bool?)c.block_ready:null;c.AssessBlock(timedOut);
   // Files only after completion/readback/stop. Never pad a short returned block.
   if(raw!=null && raw.Length==c.sample_count && times!=null) {
    c.raw_csv=String.Format(CultureInfo.InvariantCulture,"capture-{0:D4}.csv",acquisition);
    WriteBlockCsv(Path.Combine(directory,c.raw_csv),raw,times,c,tipRange);
    if(enableB && rawB!=null){c.raw_b_csv=String.Format(CultureInfo.InvariantCulture,"capture-{0:D4}-B.csv",acquisition);WriteBlockCsv(Path.Combine(directory,c.raw_b_csv),rawB,times,c,tipRange);}
   }
   c.completed_utc=DateTime.UtcNow.ToString("o");return c;
  }
  static void WriteBlockCsv(string path,short[] raw,int[] times,Capture c,double range) {
   using(StreamWriter writer=new StreamWriter(new FileStream(path,FileMode.CreateNew,FileAccess.Write),new UTF8Encoding(false))) {
    writer.NewLine="\n";writer.WriteLine("sample_index,offset_s,nominal_trigger_offset_s,adc,probe_tip_v,driver_time_s");
    for(int i=0;i<raw.Length;i++)writer.WriteLine(String.Format(CultureInfo.InvariantCulture,"{0},{1:R},{2},{3},{4:R},{5}",i,i*c.sample_interval_s,
     c.trigger_direction!="none"?((i-c.nominal_pretrigger_samples)*c.sample_interval_s).ToString("R",CultureInfo.InvariantCulture):"",raw[i],MathUtil.Volts(raw[i],range),
     i<times.Length?(times[i]*c.timestamp_quantum_s).ToString("R",CultureInfo.InvariantCulture):""));
   }
  }
  public Capture Stream(double duration,uint intervalNs,double tipRange,double pulseThreshold,double timeout,string directory,bool highResolutionTimer=true,bool diagnosticReadback=false) {
   Capture capture=null;
   if(highResolutionTimer && timeBeginPeriod(1)!=0)throw new InvalidOperationException("Cannot request 1 ms timer resolution");
   try {
    capture=StreamCore(duration,intervalNs,tipRange,pulseThreshold,timeout,directory,diagnosticReadback);
    capture.timer_resolution_requested_ms=highResolutionTimer?1:0;
    return capture;
   } finally {
    bool released=!highResolutionTimer || timeEndPeriod(1)==0;
    if(capture!=null)capture.timer_resolution_released=released;
    if(!released)throw new InvalidOperationException("Cannot release timer resolution request");
   }
  }
  Capture StreamCore(double duration,uint intervalNs,double tipRange,double pulseThreshold,double timeout,string directory,bool diagnosticReadback) {
   double requested=Math.Ceiling(duration/(intervalNs*1e-9));
   if(duration<=0 || intervalNs<100 || intervalNs>1000000 || requested<2 || requested>100000000 || timeout<=duration)
    throw new ArgumentException("Invalid streaming duration/interval/count/deadline");
   MathUtil.Threshold(pulseThreshold,tipRange);
   int wanted=(int)requested;double dt=intervalNs*1e-9;
   StreamRun start=Bind<StreamRun>("run_streaming_ns");
   StreamPoll poll=Bind<StreamPoll>("get_streaming_last_values");
   StreamStatus bufferStatus=Bind<StreamStatus>("overview_buffer_status");
   StreamCollector collector=new StreamCollector(wanted);
   Capture c=new Capture {acquisition=1,sampling_mode="stream",timebase=-1,requested_duration_s=duration,
    requested_sample_target=wanted,requested_sample_count=wanted,sample_interval_s=dt,sample_rate_hz=1/dt,range_v=tipRange,input_range_v=tipRange/10,
    pulse_threshold_v=pulseThreshold,trigger_direction="none",interval_source="ps2000_run_streaming_ns accepted interval; API has no separate interval readback"};
   Stopwatch clock=new Stopwatch();double lastCallback=0;int owner=Thread.CurrentThread.ManagedThreadId;bool insidePoll=false;
   StreamCallback callback=delegate(IntPtr buffers,short overflow,uint at,short triggered,short autoStop,uint n) {
    try {
     if(Thread.CurrentThread.ManagedThreadId!=owner || !insidePoll){collector.error="Asynchronous/reentrant callback outside polling owner";return;}
     double now=clock.Elapsed.TotalSeconds;c.max_callback_gap_s=Math.Max(c.max_callback_gap_s,now-lastCallback);lastCallback=now;
     c.driver_overflow_flags |= overflow;
     if(n>1000000) {collector.error="Invalid streaming callback size";return;}
     IntPtr a=n>0?Marshal.ReadIntPtr(buffers,0):IntPtr.Zero;
     double copyStart=clock.Elapsed.TotalSeconds;
     collector.AppendNative(a,(int)n,(overflow&1)!=0,autoStop!=0);
     double copy=clock.Elapsed.TotalSeconds-copyStart;c.max_copy_s=Math.Max(c.max_copy_s,copy);c.total_copy_s+=copy;
     if(c.callback_count==0)c.callback_samples_min=(int)n;
     c.callback_samples_min=Math.Min(c.callback_samples_min,(int)n);c.callback_samples_max=Math.Max(c.callback_samples_max,(int)n);c.callback_count++;
    } catch(Exception e) {collector.error="Streaming callback failed: "+e.Message;}
   };
   Check(trigger(handle,5,0,0,0,0),"disable streaming trigger");
   int gc0=GC.CollectionCount(0),gc1=GC.CollectionCount(1),gc2=GC.CollectionCount(2);
   Check(start(handle,intervalNs,2,(uint)wanted,1,1,(uint)c.overview_buffer_samples),"run_streaming_ns");
   c.started_utc=DateTime.UtcNow.ToString("o");clock.Start();bool timedOut=false;double lastPoll=0;
   try {
    while(!collector.autoStopped && collector.error==null) {
     double pollTime=clock.Elapsed.TotalSeconds;c.max_poll_gap_s=Math.Max(c.max_poll_gap_s,pollTime-lastPoll);lastPoll=pollTime;
     short rc;
     insidePoll=true;
     try {rc=poll(handle,callback);} finally {insidePoll=false;}
     c.polling_count++;if(rc==0)c.empty_poll_count++;
     c.max_poll_call_s=Math.Max(c.max_poll_call_s,clock.Elapsed.TotalSeconds-pollTime);
     if(rc<0){collector.error="stream poll failed";break;}
     double statusStart=clock.Elapsed.TotalSeconds;
     // Vendor acknowledged reversed return-value documentation: support topic 31221.
     short overrun;short status=bufferStatus(handle,out overrun);
     c.max_status_call_s=Math.Max(c.max_status_call_s,clock.Elapsed.TotalSeconds-statusStart);
     if(status<=0){collector.error="overview buffer status failed";break;}
     if(overrun!=0 && c.first_overrun_wall_s==null)c.first_overrun_wall_s=clock.Elapsed.TotalSeconds;
     c.buffer_overrun |= overrun!=0;
     if(c.armed_utc==null && collector.count>0) {
      c.armed_utc=DateTime.UtcNow.ToString("o");EvidenceSignal.Publish(directory,1,c.armed_utc);
     }
     if(clock.Elapsed.TotalSeconds>=timeout){timedOut=true;break;}
     if(!collector.autoStopped)Thread.Sleep(1);
    }
   } catch(Exception e) {collector.error="Acquisition exception: "+e.Message;}
   finally {
    double stopStart=clock.Elapsed.TotalSeconds;
    try {c.stop_succeeded=stop(handle)>0;} finally {c.stop_s=clock.Elapsed.TotalSeconds-stopStart;GC.KeepAlive(callback);}
   }
   c.gc0_collections=GC.CollectionCount(0)-gc0;c.gc1_collections=GC.CollectionCount(1)-gc1;c.gc2_collections=GC.CollectionCount(2)-gc2;
   c.stream_wall_s=clock.Elapsed.TotalSeconds;c.completed_utc=DateTime.UtcNow.ToString("o");
   c.sample_count=collector.count;c.auto_stopped=collector.autoStopped;c.overflow=collector.overflow;c.collector_error=collector.error;
   c.duration_s=collector.count*dt;c.sample_span_s=Math.Max(0,collector.count-1)*dt;
   short[] raw=collector.samples;if(collector.count!=raw.Length)Array.Resize(ref raw,collector.count);
   Stopwatch phase=Stopwatch.StartNew();
   if(raw.Length>0)c.analysis=MathUtil.Analyze(raw,tipRange,dt,pulseThreshold);
   c.analysis_s=phase.Elapsed.TotalSeconds;
   c.diagnostic_readback=diagnosticReadback;
   if(diagnosticReadback && c.stop_succeeded) {
    phase.Restart();
    short[] stored=new short[wanted];double first=0;short storedOverflow,storedTriggered;uint triggerAt;
    uint got=Bind<StreamReadback>("get_streaming_values_no_aggregation")(handle,ref first,stored,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,out storedOverflow,out triggerAt,out storedTriggered,(uint)wanted);
    c.readback_sample_count=(int)got;
    if(got>wanted)throw new InvalidOperationException("Invalid driver readback count");
    for(int i=0;i<(int)got;i++) {
     if(stored[i]==-32768)c.readback_lost_count++;
     if(i>=raw.Length || stored[i]!=raw[i])c.readback_mismatch_count++;
    }
    c.readback_s=phase.Elapsed.TotalSeconds;
    if(got!=c.sample_count || c.readback_mismatch_count>0 || (storedOverflow&1)!=0)c.collector_error="Driver retained readback disagrees with callback sequence";
    // Preserve the independent driver copy; never use it to fill callback gaps.
    WriteRaw(Path.Combine(directory,"driver-readback.i16le"),stored,(int)got);
   }
   c.AssessStream(collector.Complete,timedOut);
   c.raw_i16="capture-0001.i16le";c.raw_format="signed int16 little-endian, no header; ADC scaling max 32767; no aggregation; -32768 means missing data";
   if(!BitConverter.IsLittleEndian)throw new InvalidOperationException("Little-endian host required");
   phase.Restart();
   WriteRaw(Path.Combine(directory,c.raw_i16),raw,raw.Length);
   c.writer_s=phase.Elapsed.TotalSeconds;
   return c;
  }
  static void WriteRaw(string path,short[] raw,int length) {
   byte[] bytes=new byte[1048576];
   using(FileStream file=new FileStream(path,FileMode.CreateNew,FileAccess.Write)) {
    for(int offset=0;offset<length;) {
     int count=Math.Min(bytes.Length/2,length-offset);Buffer.BlockCopy(raw,offset*2,bytes,0,count*2);file.Write(bytes,0,count*2);offset+=count;
    }
    file.Flush(true);
   }
  }
  public void Dispose() {
   short rc=1;
   try {
    if(handle>0) {
     short h=handle;handle=0;
     try { if(generatorConfigured) Check(signal(h,0,0,5,1,1,0,1,0,0),"signal generator return to DC zero"); }
     finally { rc=close(h); }
    }
   }
   finally { if(module!=IntPtr.Zero) {FreeLibrary(module);module=IntPtr.Zero;} if(owns){mutex.ReleaseMutex();owns=false;} if(mutex!=null){mutex.Dispose();mutex=null;} }
   if(rc<=0) throw new InvalidOperationException("ps2000_close_unit failed");
  }
 }
}
