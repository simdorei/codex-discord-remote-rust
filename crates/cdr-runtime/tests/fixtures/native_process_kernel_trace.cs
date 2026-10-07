using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// Test-only, x64 Windows ABI. No permanent logger, file, driver, or privilege changes.
public static class CdrKernelNative {
    public const string SessionName = "CDR-QA-Process-Comparison-5060";
    public static readonly Guid ProcessGuid = new Guid("3d6fa8d0-fe05-11d0-9dda-00c04fd7ba7c");
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    public delegate void Callback(IntPtr record);
    [StructLayout(LayoutKind.Sequential)]
    public struct Descriptor { public ushort Id; public byte Version, Channel, Level, Opcode; public ushort Task; public ulong Keyword; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Header {
        public ushort Size, HeaderType, Flags, EventProperty;
        public uint ThreadId, ProcessId; public long Timestamp; public Guid ProviderId;
        public Descriptor Descriptor; public ulong ProcessorTime; public Guid ActivityId;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Record {
        public Header Header; public uint BufferContext; public ushort ExtendedDataCount, UserDataLength;
        public IntPtr ExtendedData, UserData, UserContext;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Wnode {
        public uint BufferSize, ProviderId; public ulong HistoricalContext; public long Timestamp;
        public Guid Guid; public uint ClientContext, Flags;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Properties {
        public Wnode Wnode;
        public uint BufferSize, MinimumBuffers, MaximumBuffers, MaximumFileSize, LogFileMode, FlushTimer, EnableFlags;
        public int AgeLimit;
        public uint NumberOfBuffers, FreeBuffers, EventsLost, BuffersWritten, LogBuffersLost, RealTimeBuffersLost;
        public IntPtr LoggerThreadId; public uint LogFileNameOffset, LoggerNameOffset;
    }
    // EVENT_TRACE_LOGFILEW on x64: 32 + EVENT_TRACE(88) + TRACE_LOGFILE_HEADER(280).
    [StructLayout(LayoutKind.Explicit, Size = 448)]
    public struct Logfile {
        [FieldOffset(8)] public IntPtr LoggerName;
        [FieldOffset(28)] public uint ProcessTraceMode;
        [FieldOffset(424)] public IntPtr EventRecordCallback;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct PropertyDescriptor { public ulong Name; public uint ArrayIndex, Reserved; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Context { public ulong Value; public uint Type, Size; }
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    public static extern uint StartTraceW(out ulong handle, string name, IntPtr properties);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    public static extern uint ControlTraceW(ulong handle, string name, IntPtr properties, uint code);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    public static extern ulong OpenTraceW(ref Logfile logfile);
    [DllImport("advapi32.dll", ExactSpelling = true)]
    public static extern uint ProcessTrace([In] ulong[] handles, uint count, IntPtr start, IntPtr end);
    [DllImport("advapi32.dll", ExactSpelling = true)]
    public static extern uint CloseTrace(ulong handle);
    [DllImport("tdh.dll", ExactSpelling = true)]
    public static extern uint TdhGetEventInformation(IntPtr record, uint count, ref Context context, IntPtr info, ref uint size);
    [DllImport("tdh.dll", ExactSpelling = true)]
    public static extern uint TdhGetPropertySize(IntPtr record, uint count, ref Context context, uint propertyCount, ref PropertyDescriptor property, out uint size);
    [DllImport("tdh.dll", ExactSpelling = true)]
    public static extern uint TdhGetProperty(IntPtr record, uint count, ref Context context, uint propertyCount, ref PropertyDescriptor property, uint size, [Out] byte[] value);
    public static void Check(uint status, string operation) {
        if (status != 0) throw new InvalidOperationException(operation + " status=" + status);
    }
    public static bool AbiValid() {
        return IntPtr.Size == 8 && Marshal.SizeOf(typeof(Descriptor)) == 16 &&
            Marshal.SizeOf(typeof(Header)) == 80 && Marshal.SizeOf(typeof(Record)) == 112 &&
            Marshal.SizeOf(typeof(Wnode)) == 48 && Marshal.SizeOf(typeof(Properties)) == 120 &&
            Marshal.SizeOf(typeof(Logfile)) == 448 && Marshal.SizeOf(typeof(PropertyDescriptor)) == 16;
    }
}

public sealed class CdrKernelStats {
    public uint EventsLost, LogBuffersLost, RealTimeBuffersLost, NumberOfBuffers, BuffersWritten;
    public bool LossFree { get { return EventsLost == 0 && LogBuffersLost == 0 && RealTimeBuffersLost == 0; } }
}
public interface ICdrKernelApi : IDisposable {
    uint Start(out ulong handle);
    ulong Open(CdrKernelNative.Callback callback);
    uint Process(ulong handle);
    uint Stop(ulong handle, out CdrKernelStats stats);
    uint Close(ulong handle);
}
public interface ICdrKernelFlushApi { uint Flush(ulong handle); }
public sealed class CdrKernelApi : ICdrKernelApi, ICdrKernelFlushApi {
    private IntPtr properties, name;
    private readonly string sessionName;
    private readonly bool rawTimestamp;
    public CdrKernelApi() : this(CdrKernelNative.SessionName, true) {}
    public CdrKernelApi(string sessionName, bool rawTimestamp) {
        if (String.IsNullOrWhiteSpace(sessionName) || sessionName.Length > 100 ||
            !sessionName.StartsWith("CDR-QA-", StringComparison.Ordinal))
            throw new ArgumentException("Only a dedicated CDR-QA session is allowed", "sessionName");
        this.sessionName=sessionName; this.rawTimestamp=rawTimestamp;
    }
    public readonly Guid SessionGuid = Guid.NewGuid();
    public uint ClockContext { get { return rawTimestamp ? 1U : 2U; } }
    public uint TraceMode { get { return rawTimestamp ? 0x10001100U : 0x10000100U; } }
    public uint Start(out ulong handle) {
        if (!CdrKernelNative.AbiValid()) throw new PlatformNotSupportedException("ETW fixture requires the x64 Windows ABI");
        int bytes = 120 + (sessionName.Length + 1) * 2;
        properties = Marshal.AllocHGlobal(bytes);
        Marshal.Copy(new byte[bytes], 0, properties, bytes);
        var value = new CdrKernelNative.Properties();
        value.Wnode.BufferSize = (uint)bytes; value.Wnode.Guid = SessionGuid;
        value.Wnode.ClientContext = ClockContext; value.Wnode.Flags = 0x00020000;
        value.BufferSize = 64; value.MinimumBuffers = 0; value.MaximumBuffers = 0;
        value.LogFileMode = 0x02000100; // SYSTEM_LOGGER | REAL_TIME, never NT Kernel Logger.
        value.FlushTimer = 1; value.EnableFlags = 1; // PROCESS only.
        value.LoggerNameOffset = 120; value.LogFileNameOffset = 0;
        Marshal.StructureToPtr(value, properties, false);
        return CdrKernelNative.StartTraceW(out handle, sessionName, properties);
    }
    public ulong Open(CdrKernelNative.Callback callback) {
        name = Marshal.StringToHGlobalUni(sessionName);
        var logfile = new CdrKernelNative.Logfile();
        logfile.LoggerName = name;
        // Without RAW_TIMESTAMP, ProcessTrace supplies UTC FILETIME for identity matching.
        logfile.ProcessTraceMode = TraceMode;
        logfile.EventRecordCallback = Marshal.GetFunctionPointerForDelegate(callback);
        ulong handle = CdrKernelNative.OpenTraceW(ref logfile);
        if (handle == ulong.MaxValue) throw new InvalidOperationException("OpenTraceW status=" + Marshal.GetLastWin32Error());
        return handle;
    }
    public uint Process(ulong handle) { return CdrKernelNative.ProcessTrace(new ulong[] {handle}, 1, IntPtr.Zero, IntPtr.Zero); }
    public uint Stop(ulong handle, out CdrKernelStats stats) {
        uint status = CdrKernelNative.ControlTraceW(handle, null, properties, 1); // EVENT_TRACE_CONTROL_STOP.
        stats = null;
        if (status == 0) {
            var value = (CdrKernelNative.Properties)Marshal.PtrToStructure(properties, typeof(CdrKernelNative.Properties));
            stats = new CdrKernelStats {EventsLost=value.EventsLost, LogBuffersLost=value.LogBuffersLost,
                RealTimeBuffersLost=value.RealTimeBuffersLost, NumberOfBuffers=value.NumberOfBuffers, BuffersWritten=value.BuffersWritten};
        }
        return status;
    }
    public uint Flush(ulong handle) { return CdrKernelNative.ControlTraceW(handle, null, properties, 3); }
    public uint Close(ulong handle) { return CdrKernelNative.CloseTrace(handle); }
    public void Dispose() {
        if (name != IntPtr.Zero) { Marshal.FreeHGlobal(name); name=IntPtr.Zero; }
        if (properties != IntPtr.Zero) { Marshal.FreeHGlobal(properties); properties=IntPtr.Zero; }
    }
}

public sealed class CdrKernelRow {
    public string Kind, Image, EventQpc, ReceivedQpc;
    public uint ProcessId, ParentProcessId;
    public byte Version, Opcode;
    public ushort HeaderFlags;
}
public static class CdrKernelDecode {
    private sealed class Property { public string Name; public ushort Type; }
    private static string UnicodeAt(byte[] info, uint offset) {
        if (offset < 112 || offset >= info.Length || (offset & 1) != 0) throw new InvalidOperationException("Invalid schema name offset");
        int end=(int)offset;
        while (end + 1 < info.Length && (info[end] != 0 || info[end+1] != 0)) end+=2;
        if (end + 1 >= info.Length) throw new InvalidOperationException("Unterminated schema name");
        return Encoding.Unicode.GetString(info, (int)offset, end-(int)offset);
    }
    private static Dictionary<string, Property> Schema(IntPtr record, ref CdrKernelNative.Context context) {
        uint size=0;
        uint status=CdrKernelNative.TdhGetEventInformation(record, 1, ref context, IntPtr.Zero, ref size);
        if (status != 122 || size < 112 || size > 1048576) throw new InvalidOperationException("TDH schema size/status="+size+"/"+status);
        IntPtr memory=Marshal.AllocHGlobal((int)size);
        try {
            uint allocated=size;
            CdrKernelNative.Check(CdrKernelNative.TdhGetEventInformation(record,1,ref context,memory,ref size),"TDH schema");
            if (size > allocated || size < 112) throw new InvalidOperationException("TDH schema length changed");
            byte[] bytes=new byte[size]; Marshal.Copy(memory,bytes,0,(int)size);
            uint count=BitConverter.ToUInt32(bytes,100), top=BitConverter.ToUInt32(bytes,104);
            if (count > 128 || top > count || 112L+count*24L > size) throw new InvalidOperationException("TDH property table is out of bounds");
            var result=new Dictionary<string,Property>(StringComparer.Ordinal);
            for (int index=0; index<top; index++) {
                int offset=112+index*24;
                string name=UnicodeAt(bytes,BitConverter.ToUInt32(bytes,offset+4));
                if (name != "ProcessId" && name != "ParentId" && name != "ImageFileName") continue;
                uint flags=BitConverter.ToUInt32(bytes,offset);
                if ((flags & ~64U) != 0 || BitConverter.ToUInt16(bytes,offset+16) != 1)
                    throw new InvalidOperationException("Unsupported scalar property schema: "+name);
                result.Add(name,new Property {Name=name,Type=BitConverter.ToUInt16(bytes,offset+8)});
            }
            return result;
        } finally { Marshal.FreeHGlobal(memory); }
    }
    private static byte[] Read(IntPtr record, ref CdrKernelNative.Context context, Property property) {
        IntPtr name=Marshal.StringToHGlobalUni(property.Name);
        try {
            var descriptor=new CdrKernelNative.PropertyDescriptor {Name=unchecked((ulong)name.ToInt64()),ArrayIndex=uint.MaxValue};
            uint size;
            CdrKernelNative.Check(CdrKernelNative.TdhGetPropertySize(record,1,ref context,1,ref descriptor,out size),"TDH size "+property.Name);
            if (size == 0 || size > 4096) throw new InvalidOperationException("TDH property size bound: "+property.Name);
            byte[] bytes=new byte[size];
            CdrKernelNative.Check(CdrKernelNative.TdhGetProperty(record,1,ref context,1,ref descriptor,size,bytes),"TDH property "+property.Name);
            return bytes;
        } finally { Marshal.FreeHGlobal(name); }
    }
    public static string Image(byte[] bytes, ushort type) {
        string image;
        if (type == 1 && bytes.Length >= 2 && bytes.Length % 2 == 0 && bytes[bytes.Length-1] == 0 && bytes[bytes.Length-2] == 0)
            image=Encoding.Unicode.GetString(bytes,0,bytes.Length-2);
        else if (type == 2 && bytes.Length >= 1 && bytes[bytes.Length-1] == 0)
            image=Encoding.Default.GetString(bytes,0,bytes.Length-1);
        else throw new InvalidOperationException("Unsupported or unterminated image property");
        if (image.IndexOf('\0') >= 0) throw new InvalidOperationException("Embedded terminator in image property");
        return Path.GetFileName(image);
    }
    public static CdrKernelRow Decode(IntPtr record, CdrKernelNative.Record value, long received) {
        int bits=value.Header.Flags & 0x60;
        if (bits != 0x20 && bits != 0x40) throw new InvalidOperationException("Ambiguous event pointer width");
        var context=new CdrKernelNative.Context {Type=3,Value=(ulong)(bits == 0x20 ? 4 : 8)};
        var schema=Schema(record,ref context);
        foreach (string key in new string[] {"ProcessId","ParentId","ImageFileName"})
            if (!schema.ContainsKey(key)) throw new InvalidOperationException("Missing schema property: "+key);
        byte[] pid=Read(record,ref context,schema["ProcessId"]), parent=Read(record,ref context,schema["ParentId"]);
        if (schema["ProcessId"].Type != 8 || schema["ParentId"].Type != 8 || pid.Length != 4 || parent.Length != 4)
            throw new InvalidOperationException("PID property is not a uint32 scalar");
        return new CdrKernelRow {Kind=value.Header.Descriptor.Opcode == 1 ? "start" : "stop",
            ProcessId=BitConverter.ToUInt32(pid,0),ParentProcessId=BitConverter.ToUInt32(parent,0),
            Image=Image(Read(record,ref context,schema["ImageFileName"]),schema["ImageFileName"].Type),
            EventQpc=value.Header.Timestamp.ToString(),ReceivedQpc=received.ToString(),
            Version=value.Header.Descriptor.Version,Opcode=value.Header.Descriptor.Opcode,HeaderFlags=value.Header.Flags};
    }
}

public sealed class CdrKernelTap {
    private readonly ICdrKernelApi api;
    private readonly CdrKernelNative.Callback callback;
    private readonly Func<IntPtr,CdrKernelNative.Record,long,CdrKernelRow> decode;
    private readonly ManualResetEventSlim entered=new ManualResetEventSlim(false);
    private Thread consumer;
    private ulong session, trace;
    private bool closing;
    private static readonly List<CdrKernelTap> unresolved=new List<CdrKernelTap>();
    public readonly ConcurrentQueue<CdrKernelRow> Rows=new ConcurrentQueue<CdrKernelRow>();
    public readonly ConcurrentQueue<string> Faults=new ConcurrentQueue<string>();
    public readonly List<string> CleanupErrors=new List<string>();
    public readonly ConcurrentDictionary<string,int> Schemas=new ConcurrentDictionary<string,int>();
    public bool SessionOwned, ConsumerOpened, ConsumerStarted, Joined, ApiDisposed;
    public long StartStatus=-1, StopStatus=-1, CloseStatus=-1, ProcessStatus=-1;
    public int RawCount, ProcessCount, LifecycleCount, RundownCount, Dropped, DecodeErrors;
    public int SchemaOverflow, DecodeFailures;
    public CdrKernelStats Stats;
    public CdrKernelTap(ICdrKernelApi api) : this(api,CdrKernelDecode.Decode) {}
    internal CdrKernelTap(ICdrKernelApi api, Func<IntPtr,CdrKernelNative.Record,long,CdrKernelRow> decode) {
        this.api=api; this.decode=decode; callback=Receive;
    }
    private void Receive(IntPtr record) {
        long received=Stopwatch.GetTimestamp();
        Interlocked.Increment(ref RawCount);
        try {
            var value=(CdrKernelNative.Record)Marshal.PtrToStructure(record,typeof(CdrKernelNative.Record));
            if (value.Header.ProviderId != CdrKernelNative.ProcessGuid) return;
            Interlocked.Increment(ref ProcessCount);
            string key=value.Header.Descriptor.Version+"/"+value.Header.Descriptor.Opcode+"/"+value.Header.Flags;
            if (Schemas.Count >= 128 && !Schemas.ContainsKey(key)) {
                Interlocked.Increment(ref SchemaOverflow);
                throw new InvalidOperationException("Schema counter bound exceeded");
            }
            Schemas.AddOrUpdate(key,1,delegate(string ignored,int old) {return old+1;});
            byte opcode=value.Header.Descriptor.Opcode;
            if (opcode != 1 && opcode != 2) { Interlocked.Increment(ref RundownCount); return; }
            if (Interlocked.Increment(ref LifecycleCount) > 4096) { Interlocked.Increment(ref Dropped); return; }
            try { Rows.Enqueue(decode(record,value,received)); }
            catch { Interlocked.Increment(ref DecodeFailures); throw; }
        } catch (Exception error) {
            if (Interlocked.Increment(ref DecodeErrors) <= 64) Faults.Enqueue(error.GetType().Name+": "+error.Message);
        }
    }
    public void Start() {
        StartStatus=api.Start(out session);
        CdrKernelNative.Check((uint)StartStatus,"StartTraceW");
        SessionOwned=true;
        trace=api.Open(callback); ConsumerOpened=true;
        consumer=new Thread(delegate() {
            entered.Set();
            try { Interlocked.Exchange(ref ProcessStatus,api.Process(trace)); }
            catch (Exception error) { Faults.Enqueue("ProcessTrace: "+error.Message); Interlocked.Exchange(ref ProcessStatus,-2); }
        });
        consumer.IsBackground=true; consumer.Start(); ConsumerStarted=true;
        if (!entered.Wait(500) || Interlocked.Read(ref ProcessStatus) != -1)
            throw new InvalidOperationException("ETW consumer did not remain active before helper creation");
    }
    public void Flush() {
        if (!SessionOwned || closing) throw new InvalidOperationException("No live owned ETW session to flush");
        var flusher=api as ICdrKernelFlushApi;
        if (flusher == null) throw new InvalidOperationException("ETW API does not support owned-session flush");
        CdrKernelNative.Check(flusher.Flush(session),"owned session FLUSH");
    }
    private void Cleanup(string name, Action operation) {
        try { operation(); } catch (Exception error) { CleanupErrors.Add(name+": "+error.Message); }
    }
    public void Complete() {
        if (closing) return; closing=true;
        if (SessionOwned) Cleanup("StopTrace",delegate() {
            StopStatus=api.Stop(session,out Stats); CdrKernelNative.Check((uint)StopStatus,"owned session STOP");
        });
        if (ConsumerStarted) Cleanup("drain join",delegate() { Joined=consumer.Join(1000); });
        if (ConsumerOpened) Cleanup("CloseTrace",delegate() {
            CloseStatus=api.Close(trace);
            if (CloseStatus != 0 && CloseStatus != 7007) CdrKernelNative.Check((uint)CloseStatus,"consumer CloseTrace");
        });
        if (ConsumerStarted && !Joined) Cleanup("final join",delegate() { Joined=consumer.Join(1500); });
        if (!ConsumerStarted) Joined=true;
        if (!Joined) CleanupErrors.Add("ProcessTrace thread exit remains unconfirmed");
        if (Joined) {
            Cleanup("API Dispose",delegate() { api.Dispose(); ApiDisposed=true; });
            Cleanup("signal Dispose",delegate() { entered.Dispose(); });
        } else {
            // Keep native callback and its memory alive; no future diagnostic is authorized.
            lock (unresolved) { unresolved.Add(this); }
        }
    }
    public bool Safe { get { return (!SessionOwned || StopStatus == 0) && (!ConsumerOpened || CloseStatus == 0 || CloseStatus == 7007) && Joined && ApiDisposed && CleanupErrors.Count == 0; } }
    public bool CompleteCapture { get { return Safe && SessionOwned && ConsumerOpened && ProcessStatus == 0 && Stats != null && Stats.LossFree && DecodeErrors == 0 && Dropped == 0 && Faults.IsEmpty; } }
}

// Output is bounded while the pipe continues to drain, so overflow cannot deadlock the child.
public sealed class CdrKernelText {
    private readonly StreamReader reader;
    private readonly Thread thread;
    private readonly StringBuilder text=new StringBuilder();
    public string Error;
    public bool Overflow, Joined;
    public CdrKernelText(StreamReader reader) {
        this.reader=reader;
        thread=new Thread(delegate() {
            try {
                char[] buffer=new char[4096]; int read;
                while ((read=reader.Read(buffer,0,buffer.Length)) != 0) {
                    if (text.Length+read <= 2097152 && !Overflow) text.Append(buffer,0,read);
                    else Overflow=true;
                }
            } catch (Exception error) { Error=error.Message; }
        });
        thread.IsBackground=true; thread.Start();
    }
    public void Complete() {
        Joined=thread.Join(500);
        if (!Joined) {
            try { reader.Close(); } catch (Exception error) { Error=(Error ?? "")+"; close: "+error.Message; }
            Joined=thread.Join(500);
        }
    }
    public string Text { get { if (!Joined) throw new InvalidOperationException("Output reader still active"); return text.ToString(); } }
}

public static class CdrKernelRules {
    public static string Window(long received, long begin, long frequency) {
        if (frequency <= 0 || received < begin) return "before";
        decimal ms=((decimal)received-begin)*1000/frequency;
        return ms < 5000 ? "original" : ms < 20000 ? "tail" : "cleanup";
    }
    public static bool Identity(CdrKernelRow row,uint pid,uint parent,string name,long before,long after,long exit) {
        long tick;
        return long.TryParse(row.EventQpc,out tick) && row.ProcessId == pid && row.ParentProcessId == parent &&
            string.Equals(row.Image,name,StringComparison.OrdinalIgnoreCase) &&
            tick >= before && tick <= (row.Kind == "start" ? after : exit) &&
            ((row.Kind == "start" && row.Opcode == 1) || (row.Kind == "stop" && row.Opcode == 2));
    }
}

public sealed class CdrKernelFake : ICdrKernelApi {
    private readonly string fault;
    private readonly ManualResetEventSlim done=new ManualResetEventSlim(false);
    public int StopCalls, CloseCalls, DisposeCalls;
    public CdrKernelFake(string fault) {this.fault=fault;}
    public uint Start(out ulong handle) {handle=17;return fault == "collision" ? 183U : 0U;}
    public ulong Open(CdrKernelNative.Callback callback) {if(fault == "open")throw new InvalidOperationException("injected open failure");return 18;}
    public uint Process(ulong handle) {done.Wait();return 0;}
    public uint Stop(ulong handle,out CdrKernelStats stats) {StopCalls++;done.Set();stats=new CdrKernelStats();return fault == "stop" || fault == "both" ? 5U : 0U;}
    public uint Close(ulong handle) {CloseCalls++;done.Set();return fault == "close" || fault == "both" ? 6U : fault == "pending" ? 7007U : 0U;}
    public void Dispose() {DisposeCalls++;done.Dispose();}
}
public static class CdrKernelContracts {
    private static object Row(string name,bool pass) {return new Dictionary<string,object>{{"name",name},{"pass",pass}};}
    public static object[] Run() {
        var rows=new List<object>();
        rows.Add(Row("x64_native_abi",CdrKernelNative.AbiValid()));
        foreach(string fault in new string[]{"none","collision","open","stop","close","both","pending"}) {
            var api=new CdrKernelFake(fault);var tap=new CdrKernelTap(api);string primary=null;
            try{tap.Start();}catch(Exception error){primary=error.Message;}finally{tap.Complete();}
            bool expectedSafe=fault == "none" || fault == "collision" || fault == "open" || fault == "pending";
            bool pass=tap.Safe == expectedSafe && tap.Joined && api.DisposeCalls == 1 &&
                api.StopCalls == (fault == "collision" ? 0 : 1) && api.CloseCalls == (fault == "collision" || fault == "open" ? 0 : 1) &&
                ((primary != null) == (fault == "collision" || fault == "open")) &&
                (fault != "both" || tap.CleanupErrors.Count == 2);
            rows.Add(Row("ownership_"+fault,pass));
        }
        long[] times={-1,0,4999,5000,19999,20000};string[] windows={"before","original","original","tail","tail","cleanup"};
        for(int i=0;i<times.Length;i++) rows.Add(Row("window_"+times[i],CdrKernelRules.Window(times[i],0,1000)==windows[i]));
        rows.Add(Row("invalid_frequency",CdrKernelRules.Window(1,0,0)=="before"));
        var value=new CdrKernelRow{Kind="start",Opcode=1,ProcessId=22,ParentProcessId=11,Image="cmd.exe",EventQpc="150"};
        rows.Add(Row("payload_identity",CdrKernelRules.Identity(value,22,11,"cmd.exe",100,200,400)));
        rows.Add(Row("wrong_pid_rejected",!CdrKernelRules.Identity(value,11,11,"cmd.exe",100,200,400)));
        rows.Add(Row("wrong_parent_rejected",!CdrKernelRules.Identity(value,22,99,"cmd.exe",100,200,400)));
        rows.Add(Row("outside_start_call_rejected",!CdrKernelRules.Identity(value,22,11,"cmd.exe",160,200,400)));
        value.Opcode=3;rows.Add(Row("rundown_rejected",!CdrKernelRules.Identity(value,22,11,"cmd.exe",100,200,400)));
        rows.Add(Row("ansi_image",CdrKernelDecode.Image(Encoding.ASCII.GetBytes("cmd.exe\0"),2)=="cmd.exe"));
        rows.Add(Row("unicode_image",CdrKernelDecode.Image(Encoding.Unicode.GetBytes("cmd.exe\0"),1)=="cmd.exe"));
        bool rejected=false;try{CdrKernelDecode.Image(new byte[]{65},2);}catch(InvalidOperationException){rejected=true;}
        rows.Add(Row("unterminated_image_rejected",rejected));
        return rows.ToArray();
    }
}
