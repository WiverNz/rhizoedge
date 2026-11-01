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
 }
 public sealed class Capture {
  public int acquisition, sample_count, requested_sample_target, timebase, exit_code;
  public double requested_duration_s, sample_interval_s, sample_rate_hz, duration_s, sample_span_s;
  public double requested_pretrigger_s, nominal_pretrigger_samples;
  public double range_v, input_range_v, pulse_threshold_v;
  public double? trigger_threshold_v, trigger_threshold_actual_v;
  public short? trigger_threshold_adc;
  public bool? triggered;
  public bool overflow, valid;
  public string trigger_direction, armed_utc, completed_utc, raw_csv, status;
  public Analysis analysis;
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
  public static Analysis Analyze(short[] samples, double tipRange, double interval, double threshold) {
   if (samples.Length==0 || interval<=0) throw new ArgumentException("Nonempty samples and positive interval required.");
   Analysis a=new Analysis(); double min=Double.PositiveInfinity,max=Double.NegativeInfinity;
   int start=-1;
   for(int i=0;i<samples.Length;i++) {
    double v=Volts(samples[i],tipRange); min=Math.Min(min,v); max=Math.Max(max,v);
    a.clipped |= Math.Abs((int)samples[i])>=32767; a.lost_data |= samples[i]==-32768;
    if(v>threshold && start<0) start=i;
    if(v<=threshold && start>=0) { AddPulse(a,start,i,samples.Length,interval,false); start=-1; }
   }
   if(start>=0) AddPulse(a,start,samples.Length-1,samples.Length,interval,true);
   a.min_v=min; a.max_v=max; a.peak_to_peak_v=max-min; a.pulse_count=a.pulses.Count;
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
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int Values(short h,[Out] short[] a,IntPtr b,IntPtr c,IntPtr d,out short overflow,int n);
  [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate short Signal(short h,int offset,uint peakToPeak,int wave,float start,float end,float increment,float dwell,int sweep,uint sweeps);
  Signal signal; bool generatorConfigured;
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
  public Capture Acquire(int acquisition,double duration,int target,double tipRange,string direction,double threshold,double pulseThreshold,double pre,double timeout,string directory) {
   if(duration<=0 || target<2 || target>1000000 || timeout<=duration || pre<0 || pre>=duration) throw new ArgumentException("Invalid duration, target, pretrigger or timeout.");
   MathUtil.Threshold(pulseThreshold,tipRange);
   short selected=-1; int interval=0,n=0;
   for(short tb=0;tb<=23;tb++) {
    int dt,max; short units;
    // time_interval is always nanoseconds; units is only a get_times hint.
    if(timebase(handle,tb,2,out dt,out units,1,out max)<=0 || dt<=0) continue;
    double needed=Math.Ceiling(duration/(dt*1e-9));
    if(needed>=2 && needed<=Math.Min(target,max)) { selected=tb;interval=dt;n=(int)needed;break; }
   }
   if(selected<0) throw new InvalidOperationException("No timebase meets duration/sample target; shorten duration or increase target.");
   double seconds=interval*1e-9;
   bool triggeredMode=direction!="none";
   float delay=triggeredMode ? (float)(-100.0*pre/(n*seconds)) : 0;
   Check(trigger(handle,triggeredMode?(short)0:(short)5,triggeredMode?MathUtil.Threshold(threshold,tipRange):(short)0,direction=="falling"?(short)1:(short)0,delay,0),"set_trigger2");
   Capture c=new Capture { acquisition=acquisition,requested_duration_s=duration,requested_sample_target=target,
    timebase=selected,sample_interval_s=seconds,sample_rate_hz=1/seconds,range_v=tipRange,input_range_v=tipRange/10,
    trigger_direction=direction,trigger_threshold_v=triggeredMode?(double?)threshold:null,pulse_threshold_v=pulseThreshold,
    trigger_threshold_adc=triggeredMode?(short?)MathUtil.Threshold(threshold,tipRange):null,
    trigger_threshold_actual_v=triggeredMode?(double?)MathUtil.Volts(MathUtil.Threshold(threshold,tipRange),tipRange):null,
    requested_pretrigger_s=pre,nominal_pretrigger_samples=triggeredMode?-delay*n/100.0:0 };
   int ms; Check(run(handle,n,selected,1,out ms),"run_block");
   c.armed_utc=DateTime.UtcNow.ToString("o");
   File.WriteAllText(Path.Combine(directory,"armed.txt"),acquisition+" "+c.armed_utc);
   Stopwatch clock=Stopwatch.StartNew();
   try {
    while(true) {
     short r=ready(handle); if(r<0) throw new InvalidOperationException("ps2000_ready failed"); if(r>0) break;
     if(clock.Elapsed.TotalSeconds>=timeout) {
      c.triggered=triggeredMode?(bool?)false:null; c.status="CAPTURE TIMEOUT";c.exit_code=22;c.valid=false;
      c.completed_utc=DateTime.UtcNow.ToString("o");return c;
     }
     Thread.Sleep(2);
    }
    short[] raw=new short[n]; short overflow;
    int got=values(handle,raw,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,out overflow,n);
    if(got!=n) throw new InvalidOperationException("Incomplete data: "+got+" of "+n);
    c.sample_count=n;c.duration_s=n*seconds;c.sample_span_s=(n-1)*seconds;c.overflow=(overflow&1)!=0;
    c.triggered=triggeredMode?(bool?)true:null;c.analysis=MathUtil.Analyze(raw,tipRange,seconds,pulseThreshold);
    c.valid=!c.overflow&&!c.analysis.clipped&&!c.analysis.lost_data;c.status=c.valid?"CAPTURED":"INVALID CAPTURE";c.exit_code=c.valid?0:23;
    c.raw_csv=String.Format(CultureInfo.InvariantCulture,"capture-{0:D4}.csv",acquisition);
    string path=Path.Combine(directory,c.raw_csv);
    using(StreamWriter writer=new StreamWriter(new FileStream(path,FileMode.CreateNew,FileAccess.Write),new UTF8Encoding(false))) {
     writer.NewLine="\n";writer.WriteLine("sample_index,offset_s,nominal_trigger_offset_s,adc,probe_tip_v");
     for(int i=0;i<n;i++) writer.WriteLine(String.Format(CultureInfo.InvariantCulture,"{0},{1:R},{2},{3},{4:R}",i,i*seconds,
      triggeredMode?((i-c.nominal_pretrigger_samples)*seconds).ToString("R",CultureInfo.InvariantCulture):"",raw[i],MathUtil.Volts(raw[i],tipRange)));
    }
    c.completed_utc=DateTime.UtcNow.ToString("o");return c;
   } finally { Check(stop(handle),"stop"); }
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
