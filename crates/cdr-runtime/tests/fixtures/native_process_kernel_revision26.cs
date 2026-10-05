using System;
using System.Collections.Generic;
using System.Reflection;
using System.Runtime.InteropServices;

// Synthetic ingress only. No ETW session is opened and no real TDH success is claimed.
public static class CdrKernelIngressContracts {
    private static object Check(string name, bool pass) { return new Dictionary<string,object>{{"name",name},{"pass",pass}}; }
    private static int Counter(CdrKernelTap tap,string name) {
        FieldInfo field=typeof(CdrKernelTap).GetField(name);
        return field == null ? -1 : (int)field.GetValue(tap);
    }
    private static int Total(CdrKernelTap tap) { int result=0;foreach(var pair in tap.Schemas)result+=pair.Value;return result; }
    private static CdrKernelTap NewTap(Func<IntPtr,CdrKernelNative.Record,long,CdrKernelRow> decode,out bool injected) {
        var types=new Type[]{typeof(ICdrKernelApi),typeof(Func<IntPtr,CdrKernelNative.Record,long,CdrKernelRow>)};
        var constructor=typeof(CdrKernelTap).GetConstructor(BindingFlags.Instance|BindingFlags.NonPublic,null,types,null);
        var api=new CdrKernelFake("none");injected=constructor != null;
        return injected ? (CdrKernelTap)constructor.Invoke(new object[]{api,decode}) : new CdrKernelTap(api);
    }
    private static CdrKernelRow Decode(IntPtr pointer,CdrKernelNative.Record value,long received) {
        return new CdrKernelRow {Kind=value.Header.Descriptor.Opcode == 1 ? "start" : "stop",Opcode=value.Header.Descriptor.Opcode,
            Version=value.Header.Descriptor.Version,HeaderFlags=value.Header.Flags,ProcessId=73,ParentProcessId=37,Image="cmd.exe",
            EventQpc=value.Header.Timestamp.ToString(),ReceivedQpc=received.ToString()};
    }
    private static void Feed(CdrKernelTap tap,byte version,byte opcode,int length) {
        var value=new CdrKernelNative.Record();value.Header.ProviderId=CdrKernelNative.ProcessGuid;
        value.Header.Flags=0x40;value.Header.Descriptor.Version=version;value.Header.Descriptor.Opcode=opcode;
        value.Header.Timestamp=opcode == 2 ? 250 : 150;value.UserDataLength=(ushort)length;
        IntPtr memory=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(CdrKernelNative.Record)));
        try {
            Marshal.StructureToPtr(value,memory,false);
            typeof(CdrKernelTap).GetMethod("Receive",BindingFlags.NonPublic|BindingFlags.Instance).Invoke(tap,new object[]{memory});
        } finally { Marshal.FreeHGlobal(memory); }
    }
    public static object[] Run() {
        var checks=new List<object>();bool injected;int decoded=0;
        var tap=NewTap(delegate(IntPtr pointer,CdrKernelNative.Record record,long received){decoded++;return Decode(pointer,record,received);},out injected);
        try {
            tap.Start();
            for(int length=1;length<=129;length++)Feed(tap,4,3,length);
            checks.Add(Check("same_schema_variable_lengths",tap.Schemas.Count == 1 && tap.RundownCount == 129 && tap.DecodeErrors == 0));
            // The unmodified V25 guard rejects these new opcode keys before native Decode.
            if(injected || tap.Schemas.Count == 128){Feed(tap,4,1,160);Feed(tap,4,2,164);}
            checks.Add(Check("lifecycle_after_variable_lengths",injected && decoded == 2 && tap.LifecycleCount == 2 && tap.Rows.Count == 2 && tap.DecodeErrors == 0));
            var rows=tap.Rows.ToArray();long tick;
            checks.Add(Check("lifecycle_identity_qpc_preserved",rows.Length == 2 && rows[0].ProcessId == 73 && rows[0].ParentProcessId == 37 &&
                rows[0].Image == "cmd.exe" && rows[0].Kind == "start" && rows[0].EventQpc == "150" && rows[1].Kind == "stop" && rows[1].EventQpc == "250" &&
                long.TryParse(rows[0].ReceivedQpc,out tick) && tick > 0));
        } finally {tap.Complete();}
        checks.Add(Check("valid_ingress_capture",injected && tap.Safe && tap.CompleteCapture && Counter(tap,"SchemaOverflow") == 0 && Counter(tap,"DecodeFailures") == 0));
        tap=NewTap(Decode,out injected);
        try {
            tap.Start();for(int version=0;version<=128;version++)Feed(tap,(byte)version,3,100);
            checks.Add(Check("genuine_schema_overflow",tap.Schemas.Count == 128 && Counter(tap,"SchemaOverflow") == 1 && tap.DecodeErrors == 1));
            Feed(tap,0,3,2000);
            checks.Add(Check("existing_schema_survives_overflow",Total(tap) == 129 && tap.Schemas.Count == 128 && tap.DecodeErrors == 1));
        } finally {tap.Complete();}
        checks.Add(Check("schema_overflow_capture_inconclusive",tap.Safe && !tap.CompleteCapture && Counter(tap,"DecodeFailures") == 0));
        tap=NewTap(delegate(IntPtr pointer,CdrKernelNative.Record record,long received){throw new InvalidOperationException("injected malformed payload");},out injected);
        try {
            tap.Start();if(injected)for(int index=0;index<70;index++)Feed(tap,4,1,80);
            checks.Add(Check("decoder_error_recorded",injected && Counter(tap,"DecodeFailures") == 70 && Counter(tap,"SchemaOverflow") == 0 && tap.Rows.Count == 0));
            checks.Add(Check("decoder_errors_bounded",injected && tap.DecodeErrors == 70 && tap.Faults.Count == 64));
        } finally {tap.Complete();}
        checks.Add(Check("decoder_failure_cleanup_separate",injected && tap.Safe && !tap.CompleteCapture));
        tap=NewTap(Decode,out injected);
        try {tap.Start();if(injected)for(int index=0;index<4097;index++)Feed(tap,4,1,80);}
        finally {tap.Complete();}
        checks.Add(Check("ingress_row_bound",injected && tap.Rows.Count == 4096 && tap.Dropped == 1 && tap.Safe && !tap.CompleteCapture));
        return checks.ToArray();
    }
}
