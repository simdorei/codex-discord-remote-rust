using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Management;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public sealed class CdrRouteRow {
    public string Kind;
    public uint ProcessId;
    public uint ParentProcessId;
    public string Name;
    public string CreatedTick;
    public string ReceivedUtcTick;
    public long ReceivedMs;
}

public sealed class CdrRouteTap : IDisposable {
    public readonly ConcurrentQueue<CdrRouteRow> Rows = new ConcurrentQueue<CdrRouteRow>();
    public readonly ConcurrentQueue<string> Faults = new ConcurrentQueue<string>();
    public readonly ConcurrentQueue<string> CleanupFaults = new ConcurrentQueue<string>();
    private readonly Stopwatch clock;
    private ManagementEventWatcher start;
    private ManagementEventWatcher stop;
    private int rowCount;
    private int failed;
    private int closing;

    public CdrRouteTap(Stopwatch clock) { this.clock = clock; }
    private void Fail(string message) {
        if (Interlocked.Exchange(ref failed, 1) == 0) Faults.Enqueue(message);
    }
    private ManagementEventWatcher Create(string kind, string className) {
        var watcher = new ManagementEventWatcher(
            new ManagementScope(@"\\.\root\cimv2"),
            new WqlEventQuery("SELECT * FROM " + className), new EventWatcherOptions());
        watcher.EventArrived += delegate(object sender, EventArrivedEventArgs args) {
            try {
                long received = clock.ElapsedMilliseconds;
                string utc = DateTime.UtcNow.ToFileTimeUtc().ToString(CultureInfo.InvariantCulture);
                if (Interlocked.Increment(ref rowCount) > 4096) {
                    Fail("direct raw row limit exceeded");
                    return;
                }
                var value = args.NewEvent;
                Rows.Enqueue(new CdrRouteRow {
                    Kind = kind,
                    ProcessId = Convert.ToUInt32(value["ProcessID"], CultureInfo.InvariantCulture),
                    ParentProcessId = Convert.ToUInt32(value["ParentProcessID"], CultureInfo.InvariantCulture),
                    Name = Convert.ToString(value["ProcessName"], CultureInfo.InvariantCulture),
                    CreatedTick = Convert.ToString(value["TIME_CREATED"], CultureInfo.InvariantCulture),
                    ReceivedUtcTick = utc, ReceivedMs = received
                });
            } catch (Exception error) { Fail(error.GetType().Name + ": " + error.Message); }
        };
        watcher.Stopped += delegate(object sender, StoppedEventArgs args) {
            if (Volatile.Read(ref closing) == 0) Fail("watcher stopped during collection: " + args.Status);
        };
        return watcher;
    }
    public void Start() {
        start = Create("start", "Win32_ProcessStartTrace");
        start.Start();
        stop = Create("stop", "Win32_ProcessStopTrace");
        stop.Start();
    }
    private void Cleanup(string stage, Action action) {
        try { action(); }
        catch (Exception error) { CleanupFaults.Enqueue(stage + ": " + error.Message); }
    }
    public void Dispose() {
        Interlocked.Exchange(ref closing, 1);
        if (start != null) {
            Cleanup("start.Stop", delegate { start.Stop(); });
            Cleanup("start.Dispose", delegate { start.Dispose(); });
        }
        if (stop != null) {
            Cleanup("stop.Stop", delegate { stop.Stop(); });
            Cleanup("stop.Dispose", delegate { stop.Dispose(); });
        }
    }
}

public sealed class CdrRouteGate {
    public bool Closed;
    public bool Attempted;
    public CdrRouteRow Control;
    public bool Observe(CdrRouteRow[] queued, CdrRouteRow[] direct, string window,
                        uint observer, long nowMs) {
        if (Closed) return false;
        if (nowMs >= 1000) { Closed = true; return false; }
        Control = CdrRouteRules.Common(queued, direct, window, observer, "start", 1000);
        if (Control == null) return false;
        // Consume authorization before Process.Start, even if that call fails.
        Closed = true;
        Attempted = true;
        return true;
    }
}

public static class CdrRouteRules {
    private static bool Tick(string value, out ulong result) {
        return UInt64.TryParse(value, NumberStyles.None, CultureInfo.InvariantCulture, out result);
    }
    private static bool Fresh(CdrRouteRow row, ulong window, uint observer, string kind, long limit) {
        ulong created, received;
        return row != null && row.Kind == kind && row.ProcessId != 0 &&
            row.ProcessId != observer && row.ParentProcessId != observer &&
            !String.IsNullOrEmpty(row.Name) && row.ReceivedMs >= 0 && row.ReceivedMs < limit &&
            Tick(row.CreatedTick, out created) && Tick(row.ReceivedUtcTick, out received) &&
            created >= window && created <= received;
    }
    private static string Key(CdrRouteRow row) {
        return row.ProcessId.ToString(CultureInfo.InvariantCulture) + "|" +
            row.ParentProcessId.ToString(CultureInfo.InvariantCulture) + "|" +
            row.Name.ToUpperInvariant() + "|" + row.CreatedTick;
    }
    public static CdrRouteRow Common(CdrRouteRow[] queued, CdrRouteRow[] direct, string window,
                                    uint observer, string kind, long limit) {
        ulong begin;
        if (!Tick(window, out begin)) return null;
        var matching = new HashSet<string>(StringComparer.Ordinal);
        foreach (var row in direct) if (Fresh(row, begin, observer, kind, limit)) matching.Add(Key(row));
        foreach (var row in queued) {
            if (Fresh(row, begin, observer, kind, limit) && matching.Contains(Key(row))) return row;
        }
        return null;
    }
    public static bool Lifetime(string window, string end, string created, string exited) {
        ulong w, e, c, x;
        return Tick(window, out w) && Tick(end, out e) && Tick(created, out c) && Tick(exited, out x) &&
            w > 0 && e > w && c >= w && x > c && x <= e;
    }
}

// TOKEN_QUERY only. No token adjustment, impersonation, ACL writes or raw SID/logon export.
public static class CdrRouteContext {
    [StructLayout(LayoutKind.Sequential)]
    private struct Luid { public uint Low; public int High; }
    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool OpenThreadToken(IntPtr thread, uint access, bool asSelf, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool GetTokenInformation(IntPtr token, int kind, IntPtr data, int size, out int needed);
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool LookupPrivilegeName(string system, ref Luid id, StringBuilder name, ref int size);
    [DllImport("advapi32.dll")]
    private static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);
    [DllImport("advapi32.dll")]
    private static extern IntPtr GetSidSubAuthority(IntPtr sid, uint index);
    [DllImport("advapi32.dll")]
    private static extern bool EqualSid(IntPtr first, IntPtr second);
    [DllImport("kernel32.dll")]
    private static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")]
    private static extern IntPtr GetCurrentThread();
    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr handle);

    private static T Info<T>(IntPtr token, int kind, Func<IntPtr, int, T> read) {
        int size;
        GetTokenInformation(token, kind, IntPtr.Zero, 0, out size);
        if (size <= 0 || size > 65536) throw new Win32Exception(Marshal.GetLastWin32Error());
        IntPtr data = Marshal.AllocHGlobal(size);
        try {
            int written;
            if (!GetTokenInformation(token, kind, data, size, out written))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            if (written > size || written <= 0) throw new InvalidOperationException("invalid token buffer length");
            return read(data, written);
        } finally { Marshal.FreeHGlobal(data); }
    }
    private static int Number(IntPtr token, int kind) {
        return Info(token, kind, delegate(IntPtr data, int size) {
            if (size < 4) throw new InvalidOperationException("short token number");
            return Marshal.ReadInt32(data);
        });
    }
    public static bool DecodeHasRestrictions(byte[] value) {
        if (value == null || (value.Length != 1 && value.Length != 4))
            throw new InvalidOperationException("unsupported has_restrictions length");
        if (value.Length == 1) {
            if (value[0] > 1) throw new InvalidOperationException("invalid has_restrictions boolean");
            return value[0] != 0;
        }
        return BitConverter.ToUInt32(value, 0) != 0;
    }
    private static bool HasRestrictions(IntPtr token, Dictionary<string, object> result) {
        return Info(token, 21, delegate(IntPtr data, int size) {
            result["has_restrictions_returned_bytes"] = size;
            var value = new byte[size];
            Marshal.Copy(data, value, 0, size);
            return DecodeHasRestrictions(value);
        });
    }
    private static void Put(Dictionary<string, object> result, List<string> errors,
                            string name, Func<object> read) {
        try { result[name] = read(); }
        catch (Exception error) { result[name] = null; errors.Add(name + ": " + error.Message); }
    }
    private static object Privileges(IntPtr token) {
        return Info(token, 3, delegate(IntPtr data, int size) {
            if (size < 4) throw new InvalidOperationException("short privileges");
            int count = Marshal.ReadInt32(data);
            if (count < 0 || count > 128 || size < 4 + count * 12)
                throw new InvalidOperationException("invalid privileges length");
            var rows = new List<Dictionary<string, object>>();
            for (int index = 0; index < count; index++) {
                IntPtr item = IntPtr.Add(data, 4 + index * 12);
                var luid = (Luid)Marshal.PtrToStructure(item, typeof(Luid));
                uint attributes = unchecked((uint)Marshal.ReadInt32(item, 8));
                var name = new StringBuilder(256);
                int capacity = name.Capacity;
                if (!LookupPrivilegeName(null, ref luid, name, ref capacity))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                rows.Add(new Dictionary<string, object> {
                    { "name", name.ToString() }, { "present", true },
                    { "enabled", (attributes & 2) != 0 }, { "attributes", attributes }
                });
            }
            return rows;
        });
    }
    private static object Integrity(IntPtr token) {
        return Info(token, 25, delegate(IntPtr data, int size) {
            if (size < IntPtr.Size + 4) throw new InvalidOperationException("short integrity label");
            IntPtr sid = Marshal.ReadIntPtr(data);
            byte count = Marshal.ReadByte(GetSidSubAuthorityCount(sid));
            if (count == 0 || count > 15) throw new InvalidOperationException("invalid integrity SID");
            return unchecked((uint)Marshal.ReadInt32(GetSidSubAuthority(sid, (uint)(count - 1))));
        });
    }
    private static object SameUser(IntPtr token, IntPtr reference) {
        return Info(token, 1, delegate(IntPtr first, int firstSize) {
            if (firstSize < IntPtr.Size + 4) throw new InvalidOperationException("short token user");
            return Info(reference, 1, delegate(IntPtr second, int secondSize) {
                if (secondSize < IntPtr.Size + 4) throw new InvalidOperationException("short reference user");
                return EqualSid(Marshal.ReadIntPtr(first), Marshal.ReadIntPtr(second));
            });
        });
    }
    private static object SameLogon(IntPtr token, IntPtr reference) {
        return Info(token, 10, delegate(IntPtr first, int firstSize) {
            if (firstSize < 16) throw new InvalidOperationException("short token statistics");
            return Info(reference, 10, delegate(IntPtr second, int secondSize) {
                if (secondSize < 16) throw new InvalidOperationException("short reference statistics");
                return Marshal.ReadInt64(first, 8) == Marshal.ReadInt64(second, 8);
            });
        });
    }
    private static Dictionary<string, object> Read(IntPtr token, string source) {
        var result = new Dictionary<string, object> {
            { "query_only", true }, { "source", source }, { "query_thread_id", GetCurrentThreadId() },
            { "queried_at", DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture) }
        };
        var errors = new List<string>();
        Put(result, errors, "session_id", delegate { return Number(token, 12); });
        Put(result, errors, "elevation_type", delegate { return Number(token, 18); });
        Put(result, errors, "elevated", delegate { return Number(token, 20) != 0; });
        result["has_restrictions_returned_bytes"] = null;
        Put(result, errors, "has_restrictions", delegate { return HasRestrictions(token, result); });
        Put(result, errors, "token_type", delegate { return Number(token, 8); });
        Put(result, errors, "integrity_rid", delegate { return Integrity(token); });
        Put(result, errors, "privileges", delegate { return Privileges(token); });
        if (source == "thread") Put(result, errors, "impersonation_level", delegate { return Number(token, 9); });
        IntPtr reference;
        if (OpenProcessToken(GetCurrentProcess(), 8, out reference)) {
            try {
                Put(result, errors, "user_equals_observer_process", delegate { return SameUser(token, reference); });
                Put(result, errors, "logon_equals_observer_process", delegate { return SameLogon(token, reference); });
            } finally {
                if (!CloseHandle(reference)) errors.Add("reference token close: " + Marshal.GetLastWin32Error());
            }
        } else errors.Add("reference token open: " + Marshal.GetLastWin32Error());
        result["errors"] = errors;
        result["status"] = errors.Count == 0 ? "ok" : "partial";
        return result;
    }
    private static Dictionary<string, object> Failure(string source, int error) {
        return new Dictionary<string, object> {
            { "query_only", true }, { "source", source }, { "win32_error", error },
            { "status", source == "thread" && error == 1008 ? "no_thread_token" : "error" }
        };
    }
    private static Dictionary<string, object> OwnedToken(IntPtr token, string source) {
        Dictionary<string, object> result = null;
        try { result = Read(token, source); return result; }
        finally {
            if (!CloseHandle(token) && result != null) {
                result["status"] = "partial";
                ((List<string>)result["errors"]).Add("token close: " + Marshal.GetLastWin32Error());
            }
        }
    }
    public static Dictionary<string, object> Process(IntPtr handle) {
        IntPtr token;
        if (!OpenProcessToken(handle, 8, out token)) return Failure("process", Marshal.GetLastWin32Error());
        return OwnedToken(token, "process");
    }
    public static Dictionary<string, object> Observer() { return Process(GetCurrentProcess()); }
    public static Dictionary<string, object> Thread() {
        IntPtr token;
        if (!OpenThreadToken(GetCurrentThread(), 8, true, out token)) return Failure("thread", Marshal.GetLastWin32Error());
        return OwnedToken(token, "thread");
    }
}
