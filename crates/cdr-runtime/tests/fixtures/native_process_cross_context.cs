using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Globalization;
using System.Collections;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.ComponentModel;
using System.Diagnostics;
using System.Management;
using System.Runtime.InteropServices;
using System.Threading;
using System.Web.Script.Serialization;

// Test-only, query-only cross-lineage diagnostic. Never grants native readiness.
public static class CdrCross {
    public static long Now { get { return Stopwatch.GetTimestamp(); } }
    public static long Frequency { get { return Stopwatch.Frequency; } }
    public static string S(long value) { return value.ToString(CultureInfo.InvariantCulture); }
    public static long N(object value) { return Int64.Parse(Convert.ToString(value, CultureInfo.InvariantCulture), CultureInfo.InvariantCulture); }
    public static Dictionary<string, object> D(params object[] pairs) {
        var result = new Dictionary<string, object>();
        for (int i = 0; i < pairs.Length; i += 2) result.Add((string)pairs[i], pairs[i + 1]);
        return result;
    }
    public static object[] ArrayOf(object value) {
        if (value == null) return new object[0];
        return ((IEnumerable)value).Cast<object>().ToArray();
    }
    public static Dictionary<string, object> Map(object value) { return (Dictionary<string, object>)value; }
    public static string Text(Dictionary<string, object> value, string key) { return Convert.ToString(value[key], CultureInfo.InvariantCulture); }
    public static JavaScriptSerializer Json() { return new JavaScriptSerializer { MaxJsonLength = 2097152, RecursionLimit = 64 }; }
    public static string Serialize(object value) { return Json().Serialize(value); }
    public static bool Before(long value, long epoch, long frequency, int milliseconds) {
        return frequency > 0 && value >= epoch && (decimal)(value - epoch) * 1000 < (decimal)frequency * milliseconds;
    }
    public static bool Setup(long[] first, bool[] healthy, long dispatch, long epoch, long frequency) {
        return first != null && first.Length == 4 && healthy != null && healthy.Length == 4 &&
            Before(dispatch, epoch, frequency, 1000) &&
            first.Select((value, index) => value >= epoch && value < dispatch && healthy[index]).All(value => value);
    }
    public static bool AllowHeld(bool setup, bool start, bool stop, bool exit, long now, long epoch, long frequency) {
        return setup && start && stop && exit && Before(now, epoch, frequency, 1000);
    }
    public static bool AllowPost(bool ready, bool exit, long release, long now, long epoch, long frequency) {
        return ready && exit && release - epoch >= frequency * 5 / 2 && Before(now, epoch, frequency, 5000);
    }
    public static bool PeerEnvelope(Dictionary<string, object> value, string nonce, string pin, string role,
                                    string phase, long expiry, int pid, string created) {
        try {
            return Text(value, "nonce") == nonce && Text(value, "bundle") == pin && Text(value, "role") == role &&
                Text(value, "phase") == phase && N(value["expiry"]) == expiry && N(value["pid"]) == pid &&
                Text(value, "created") == created && N(value["sent"]) < expiry;
        } catch { return false; }
    }
    public static bool TokenEqual(Dictionary<string, object> first, Dictionary<string, object> second) {
        string[] keys = { "session_id", "elevation_type", "elevated", "has_restrictions", "token_type", "integrity_rid" };
        if (Text(first, "status") != "ok" || Text(second, "status") != "ok") return false;
        if (!(bool)first["user_equals_observer_process"] || !(bool)first["logon_equals_observer_process"] ||
            !(bool)second["user_equals_observer_process"] || !(bool)second["logon_equals_observer_process"]) return false;
        foreach (string key in keys) if (Text(first, key) != Text(second, key)) return false;
        Func<Dictionary<string, object>, string> privileges = delegate(Dictionary<string, object> token) {
            return String.Join("|", ArrayOf(token["privileges"]).Select(item => {
                var row = Map(item); return Text(row, "name") + ":" + Text(row, "attributes");
            }).OrderBy(item => item, StringComparer.Ordinal).ToArray());
        };
        return privileges(first) == privileges(second);
    }
    public static bool EffectiveThread(Dictionary<string, object> thread, Dictionary<string, object> process) {
        return Text(thread, "status") == "no_thread_token" || TokenEqual(thread, process);
    }
    public static Dictionary<string, object> ErrorEvidence(Exception error) {
        return D("type", error.GetType().Name, "message", error.Message,
            "secondary_close_error", error.Data.Contains("secondary_close_error") ? error.Data["secondary_close_error"] : null);
    }
    public static string ErrorText(Exception error) { return Serialize(ErrorEvidence(error)); }
    public static void PublicationFailure(Dictionary<string, object> result, Exception error) {
        result["safe_for_follow_up"] = false;
        result["publication_error"] = ErrorEvidence(error);
    }
    public static void Attempt(List<string> errors, string stage, Action action) {
        try { action(); } catch (Exception error) { errors.Add(stage + ": " + ErrorText(error)); }
    }
    public static void Atomic(string path, string text, string inject) {
        string temporary = path + "." + Guid.NewGuid().ToString("N") + ".part";
        FileStream stream = null; Exception first = null;
        try {
            stream = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None);
            byte[] bytes = new UTF8Encoding(false).GetBytes(text); stream.Write(bytes, 0, bytes.Length);
            if (inject == "write" || inject == "both") throw new IOException("injected write failure");
            stream.Flush(true);
        } catch (Exception error) { first = error; }
        finally {
            if (stream != null) {
                try {
                    stream.Dispose();
                    if (inject == "close" || inject == "both") throw new IOException("injected close failure");
                } catch (Exception error) {
                    if (first == null) first = error; else first.Data["secondary_close_error"] = error.Message;
                }
            }
        }
        if (first != null) throw first;
        File.Move(temporary, path); // No overwrite, and only after successful close.
    }
    public static Dictionary<string, object> Read(string path) {
        var info = new FileInfo(path);
        if (info.Length > 2097152) throw new InvalidDataException("IPC file exceeds cap");
        return Json().Deserialize<Dictionary<string, object>>(File.ReadAllText(path, Encoding.UTF8));
    }
    public static string Cause(bool aa, bool ab, bool ba, bool bb) {
        if (!bb) return "INCONCLUSIVE_CONTROL";
        if (ba && !aa && !ab) return "A_OBSERVER_CONTEXT_PATH";
        if (ba && !aa && ab) return "OBSERVER_PRODUCER_RELATION";
        if (!ba && !aa && ab) return "A_PRODUCER_OR_COMMON_UPSTREAM";
        if (aa && !ba) return "B_OBSERVER_SCOPE_OR_RELATION";
        if (aa && ab && ba) return "NOT_REPRODUCED";
        return "INCONCLUSIVE_MIXED";
    }
}

public sealed class CdrCrossOnce {
    private int claimed;
    public bool Take(bool condition) { return condition && Interlocked.CompareExchange(ref claimed, 1, 0) == 0; }
}

public sealed class CdrCrossHandle : IDisposable {
    [StructLayout(LayoutKind.Sequential)]
    private struct Basic { public IntPtr Exit, Peb, Affinity, Priority, Id, Parent; }
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool GetProcessTimes(IntPtr handle, out long created, out long exited, out long kernel, out long user);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool QueryFullProcessImageName(IntPtr handle, uint flags, StringBuilder name, ref int size);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern uint WaitForSingleObject(IntPtr handle, uint ms);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool GetExitCodeProcess(IntPtr handle, out uint code);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool IsProcessInJob(IntPtr handle, IntPtr job, out bool inside);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool QueryInformationJobObject(IntPtr job, int kind, IntPtr data, uint size, out uint written);
    [DllImport("ntdll.dll")] private static extern int NtQueryInformationProcess(IntPtr handle, int kind, out Basic basic, int size, out int written);
    public IntPtr Value;
    public readonly int Pid;
    public CdrCrossHandle(int pid) {
        Pid = pid; Value = OpenProcess(0x00100400, false, pid);
        if (Value == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "OpenProcess query/synchronize");
    }
    public bool Exited(uint wait) {
        uint status = WaitForSingleObject(Value, wait);
        if (status == 0) return true;
        if (status == 258) return false;
        throw new Win32Exception(Marshal.GetLastWin32Error(), "process wait");
    }
    public long[] Times() {
        long created, exited, kernel, user;
        if (!GetProcessTimes(Value, out created, out exited, out kernel, out user)) throw new Win32Exception(Marshal.GetLastWin32Error());
        return new long[] { created, exited };
    }
    public uint ExitCode() {
        uint code; if (!GetExitCodeProcess(Value, out code)) throw new Win32Exception(Marshal.GetLastWin32Error()); return code;
    }
    public Dictionary<string, object> Identity() {
        var name = new StringBuilder(32768); int length = name.Capacity;
        if (!QueryFullProcessImageName(Value, 0, name, ref length)) throw new Win32Exception(Marshal.GetLastWin32Error());
        Basic basic; int returned;
        if (NtQueryInformationProcess(Value, 0, out basic, Marshal.SizeOf(typeof(Basic)), out returned) != 0 ||
            basic.Id.ToInt64() != Pid) throw new InvalidOperationException("process ancestry query failed");
        return CdrCross.D("pid", Pid, "created", CdrCross.S(Times()[0]), "image", name.ToString(),
            "parent_pid", basic.Parent.ToInt64(), "sample_qpc", CdrCross.S(CdrCross.Now), "frequency", CdrCross.Frequency,
            "token", CdrRouteContext.Process(Value));
    }
    public static Dictionary<string, object> Job() {
        using (var self = new CdrCrossHandle(Process.GetCurrentProcess().Id)) {
            bool inside;
            if (!IsProcessInJob(self.Value, IntPtr.Zero, out inside)) throw new Win32Exception(Marshal.GetLastWin32Error());
            var result = CdrCross.D("inside_job", inside, "nested_or_common_job_identity", "UNPROVEN", "breakaway_used", false);
            if (!inside) return result;
            IntPtr data = Marshal.AllocHGlobal(65536);
            try {
                uint written;
                if (QueryInformationJobObject(IntPtr.Zero, 3, data, 65536, out written)) {
                    int count = Marshal.ReadInt32(data, 4);
                    if (count < 0 || count > (65536 - 8) / IntPtr.Size) throw new InvalidDataException("job list count");
                    var ids = new List<string>();
                    for (int i = 0; i < count; i++) ids.Add(CdrCross.S(Marshal.ReadIntPtr(data, 8 + i * IntPtr.Size).ToInt64()));
                    result["current_job_process_ids"] = ids;
                } else result["process_list_error"] = Marshal.GetLastWin32Error();
            } finally { Marshal.FreeHGlobal(data); }
            return result;
        }
    }
    public void Dispose() {
        IntPtr value = Interlocked.Exchange(ref Value, IntPtr.Zero);
        if (value != IntPtr.Zero && !CloseHandle(value)) throw new Win32Exception(Marshal.GetLastWin32Error(), "process handle close");
    }
}

public sealed class CdrCrossTap : IDisposable {
    public readonly ConcurrentQueue<Dictionary<string, object>> Rows = new ConcurrentQueue<Dictionary<string, object>>();
    public readonly ConcurrentQueue<string> Faults = new ConcurrentQueue<string>();
    public readonly ConcurrentQueue<string> CleanupErrors = new ConcurrentQueue<string>();
    public readonly long[] First = { -1, -1 };
    public readonly Dictionary<string, object>[] Contexts = new Dictionary<string, object>[2];
    public readonly bool[] Started = new bool[2];
    public readonly bool[] JoinAttempted = new bool[2];
    private readonly Thread[] threads = new Thread[2];
    private readonly long epoch;
    private readonly Action<string> fault;
    private int closing, count;
    public bool Joined;
    private Action<int> listener;
    private Action<int, Thread> starter;
    private Func<int, Thread, bool> joiner;
    private bool synthetic;
    public CdrCrossTap(long epoch, Action<string> fault) {
        this.epoch = epoch; this.fault = fault; listener = Listen;
        starter = delegate(int index, Thread thread) { thread.Start(); };
        joiner = delegate(int index, Thread thread) { return thread.Join(1000); };
    }
    public static bool EmptyPoll(ManagementStatus status) { return status == ManagementStatus.Timedout; }
    public static CdrCrossTap ForContract(string failure) {
        var tap = new CdrCrossTap(0, delegate(string error) { }); tap.synthetic = true;
        tap.listener = delegate(int index) { while (Volatile.Read(ref tap.closing) == 0) Thread.Sleep(1); };
        tap.starter = delegate(int index, Thread thread) {
            if (index == 1 && failure == "partial_start") throw new InvalidOperationException("injected second start");
            thread.Start();
        };
        tap.joiner = delegate(int index, Thread thread) {
            if (index == 0 && failure == "join_throw") throw new InvalidOperationException("injected Join");
            if (index == 0 && failure == "join_false") return false;
            return thread.Join(1000);
        };
        return tap;
    }
    public bool ReapContractThreads() {
        if (!synthetic) throw new InvalidOperationException("not a synthetic tap");
        bool all = true;
        foreach (var thread in threads) {
            if (thread != null && (thread.ThreadState & System.Threading.ThreadState.Unstarted) == 0) all &= thread.Join(1000);
        }
        return all; // Does not rewrite the actual Join result.
    }
    private void Listen(int index) {
        ManagementEventWatcher watcher = null;
        try {
            Contexts[index] = CdrRouteContext.Thread();
            string kind = index == 0 ? "start" : "stop";
            var options = new EventWatcherOptions { Timeout = TimeSpan.FromMilliseconds(100) };
            watcher = new ManagementEventWatcher(new ManagementScope(@"\\.\root\cimv2"),
                new WqlEventQuery("SELECT * FROM Win32_Process" + (index == 0 ? "Start" : "Stop") + "Trace"), options);
            while (Volatile.Read(ref closing) == 0 && CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, 5000)) {
                try {
                    using (var value = watcher.WaitForNextEvent()) {
                        long received = CdrCross.Now;
                        Interlocked.CompareExchange(ref First[index], received, -1);
                        if (Interlocked.Increment(ref count) > 4096) throw new InvalidDataException("raw row cap exceeded");
                        Rows.Enqueue(CdrCross.D("kind", kind, "pid", Convert.ToInt64(value["ProcessID"], CultureInfo.InvariantCulture),
                            "parent", Convert.ToInt64(value["ParentProcessID"], CultureInfo.InvariantCulture),
                            "name", Convert.ToString(value["ProcessName"], CultureInfo.InvariantCulture),
                            "created", Convert.ToString(value["TIME_CREATED"], CultureInfo.InvariantCulture),
                            "received_utc", CdrCross.S(DateTime.UtcNow.ToFileTimeUtc()), "received_qpc", CdrCross.S(received)));
                    }
                } catch (ManagementException error) {
                    if (!EmptyPoll(error.ErrorCode)) throw;
                    Interlocked.CompareExchange(ref First[index], CdrCross.Now, -1);
                }
            }
        } catch (Exception error) {
            string message = index + ": " + error.GetType().Name + ": " + error.Message;
            Faults.Enqueue(message); fault(message);
        } finally {
            if (watcher != null) {
                var errors = new List<string>();
                CdrCross.Attempt(errors, index + ".stop", watcher.Stop);
                CdrCross.Attempt(errors, index + ".dispose", watcher.Dispose);
                foreach (string error in errors) CleanupErrors.Enqueue(error);
            }
        }
    }
    public void Start() {
        for (int i = 0; i < 2; i++) {
            int slot = i; threads[i] = new Thread(delegate() { listener(slot); });
            threads[i].IsBackground = true; starter(i, threads[i]); Started[i] = true;
        }
    }
    public bool Ready { get { return Interlocked.Read(ref First[0]) >= epoch && Interlocked.Read(ref First[1]) >= epoch && Faults.IsEmpty; } }
    public void Dispose() {
        Interlocked.Exchange(ref closing, 1); Joined = true;
        for (int i = 0; i < 2; i++) {
            var thread = threads[i]; if (thread == null || (thread.ThreadState & System.Threading.ThreadState.Unstarted) != 0) continue;
            JoinAttempted[i] = true;
            try { if (!joiner(i, thread)) { Joined = false; CleanupErrors.Enqueue(i + ".join timed out"); } }
            catch (Exception error) { Joined = false; CleanupErrors.Enqueue(i + ".join: " + error.Message); }
        }
    }
    public Dictionary<string, object> Report() {
        return CdrCross.D("first", First.Select(CdrCross.S).ToArray(), "threads", Contexts, "rows", Rows.ToArray(),
            "faults", Faults.ToArray(), "started", Started, "join_attempted", JoinAttempted, "joined", Joined,
            "cleanup_errors", CleanupErrors.ToArray());
    }
}

public sealed class CdrCrossProbe {
    public readonly Process Process = new Process();
    public CdrCrossHandle Handle;
    public readonly Dictionary<string, object> Record;
    private System.Threading.Tasks.Task<string> ready;
    private readonly CdrCrossOnce dispatchGate = new CdrCrossOnce();
    public bool Started, Released, ExitConfirmed, Forced;
    public CdrCrossProbe(string label, int parent) {
        Record = CdrCross.D("label", label, "parent", parent, "pid", 0, "created", "0", "exited", "0",
            "dispatch", "0", "release", "0", "release_utc", "0", "ready", false, "exit_confirmed", false,
            "forced", false, "exit_code", null, "token_match", false);
    }
    public bool Dispatch(Func<long> clock, long epoch, long frequency, int limit, Func<bool> allowed, Func<bool> start) {
        // Consume even a denied dispatch. No later call may reopen this attempt.
        if (!dispatchGate.Take(true)) throw new InvalidOperationException("duplicate dispatch attempt");
        bool freshPermission = allowed();
        long stamp = clock(); Record["dispatch"] = CdrCross.S(stamp);
        if (!freshPermission || !CdrCross.Before(stamp, epoch, frequency, limit))
            throw new InvalidOperationException("dispatch boundary closed");
        return start();
    }
    public void Start(Dictionary<string, object> self, long epoch, int limit, Func<bool> allowed) {
        Process.StartInfo = new ProcessStartInfo {
            FileName = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "cmd.exe"),
            Arguments = "/d /q /c \"echo CDR_CROSS_READY&set /p CDR_CROSS_RELEASE=&exit /b 0\"",
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true,
            RedirectStandardOutput = true, RedirectStandardError = true
        };
        if (!Dispatch(delegate { return CdrCross.Now; }, epoch, CdrCross.Frequency, limit, allowed, Process.Start))
            throw new InvalidOperationException("probe Start returned false");
        Started = true; Record["pid"] = Process.Id;
        Handle = new CdrCrossHandle(Process.Id);
        Record["created"] = CdrCross.S(Handle.Times()[0]);
        var identity = Handle.Identity(); Record["identity"] = identity;
        Record["token_match"] = CdrCross.TokenEqual(CdrCross.Map(identity["token"]), CdrCross.Map(self["token"]));
        if (!(bool)Record["token_match"] || CdrCross.N(identity["parent_pid"]) != CdrCross.N(Record["parent"]) ||
            !String.Equals(CdrCross.Text(identity, "image"), Process.StartInfo.FileName, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("probe token/parent/image mismatch");
        ready = Process.StandardOutput.ReadLineAsync();
    }
    public bool Ready() {
        if (ready == null || !ready.IsCompleted) return false;
        bool good = ready.GetAwaiter().GetResult() == "CDR_CROSS_READY";
        Record["ready"] = good;
        if (!good) throw new InvalidDataException("probe READY mismatch");
        return good;
    }
    public void Release() {
        if (Released) throw new InvalidOperationException("duplicate probe release");
        Record["release"] = CdrCross.S(CdrCross.Now); Record["release_utc"] = CdrCross.S(DateTime.UtcNow.ToFileTimeUtc());
        Process.StandardInput.WriteLine(""); Process.StandardInput.Close(); Released = true;
    }
    public bool ObserveExit() {
        if (Handle == null || !Handle.Exited(0)) return false;
        long[] times = Handle.Times();
        if (CdrCross.Text(Record, "created") != CdrCross.S(times[0])) throw new InvalidDataException("probe identity changed");
        Record["exited"] = CdrCross.S(times[1]); Record["exit_code"] = Handle.ExitCode();
        Record["exit_confirmed"] = true; ExitConfirmed = true;
        return true;
    }
    public void Cleanup(List<string> errors) {
        if (Started) {
            CdrCross.Attempt(errors, "probe.stdin", delegate { Process.StandardInput.Close(); });
            CdrCross.Attempt(errors, "probe.wait", delegate { Process.WaitForExit(250); ObserveExit(); });
            CdrCross.Attempt(errors, "probe.kill", delegate {
                if (!ExitConfirmed) { Forced = true; Record["forced"] = true; Process.Kill(); }
            });
            CdrCross.Attempt(errors, "probe.final_wait", delegate {
                Process.WaitForExit(1000);
                if (!ObserveExit()) throw new InvalidOperationException("probe exit unconfirmed");
            });
        }
        if (Handle != null) CdrCross.Attempt(errors, "probe.handle", Handle.Dispose);
        CdrCross.Attempt(errors, "probe.dispose", Process.Dispose);
    }
}

public sealed class CdrCrossSession {
    private readonly string directory, nonce, bundle, role, script, imagePin, fixtures;
    private readonly Action<string, object> publicationOverride;
    private readonly long expiry;
    private readonly int pid;
    private readonly Dictionary<string, object> self;
    private readonly CdrCrossHandle selfHandle;
    private CdrCrossHandle peerHandle, providerHandle;
    private Dictionary<string, object> peer;
    private readonly List<CdrCrossProbe> probes = new List<CdrCrossProbe>();
    private readonly List<string> cleanup = new List<string>();
    private readonly object owned = new object();
    private readonly ManualResetEventSlim finished = new ManualResetEventSlim(false);
    private readonly ManualResetEventSlim cleaned = new ManualResetEventSlim(false);
    private Thread watchdog;
    private bool watchdogJoined;
    private CdrCrossTap tap;
    private long epoch, windowUtc;
    private string primary;
    private int cleanupClaim, abortSent;
    private bool launchAttempted, launchConfirmed, peerExitConfirmed, watchdogUsed;
    private readonly HashSet<string> consumed = new HashSet<string>();
    private Dictionary<string, object> setupA, setupB, pre, held, post, finalB;
    private readonly CdrCrossOnce preGate = new CdrCrossOnce(), heldGate = new CdrCrossOnce(), postGate = new CdrCrossOnce();

    public CdrCrossSession(string role, string directory, string nonce, string bundle, long expiry, string script, string imagePin, string fixtures) {
        if ((role != "A" && role != "B") || nonce.Length != 32 || bundle.Length != 64 ||
            nonce.Any(value => !Uri.IsHexDigit(value)) || bundle.Any(value => !Uri.IsHexDigit(value)))
            throw new InvalidDataException("invalid fixed diagnostic identity");
        if (!Path.IsPathRooted(directory) || Path.GetFileName(directory) != "cdr-cross-" + nonce ||
            (File.GetAttributes(directory) & FileAttributes.ReparsePoint) != 0)
            throw new InvalidDataException("unbound diagnostic directory");
        if (expiry <= CdrCross.Now || expiry - CdrCross.Now > CdrCross.Frequency * 65)
            throw new InvalidDataException("expired or invalid absolute lifetime");
        this.role = role; this.directory = directory; this.nonce = nonce; this.bundle = bundle;
        this.expiry = expiry; this.script = script; this.imagePin = imagePin; this.fixtures = fixtures;
        pid = Process.GetCurrentProcess().Id;
        selfHandle = new CdrCrossHandle(pid); self = selfHandle.Identity();
        self["job"] = CdrCrossHandle.Job();
        watchdog = new Thread(Watchdog); watchdog.IsBackground = true; watchdog.Start();
    }
    // Contracts use the real finalization path without creating observers, handles or peers.
    private CdrCrossSession(string role, string directory, Action<string, object> publisher) {
        this.role = role; this.directory = directory; publicationOverride = publisher;
        self = CdrCross.D("synthetic_contract", true);
        cleaned.Set(); watchdogJoined = true;
    }
    private string Other { get { return role == "A" ? "B" : "A"; } }
    private string PathFor(string name) { return Path.Combine(directory, name + ".json"); }
    private Dictionary<string, object> Packet(string phase, object data) {
        return CdrCross.D("nonce", nonce, "bundle", bundle, "role", role, "phase", phase,
            "expiry", CdrCross.S(expiry), "pid", pid, "created", self["created"],
            "sent", CdrCross.S(CdrCross.Now), "frequency", CdrCross.Frequency, "epoch", CdrCross.S(epoch), "data", data);
    }
    private void Put(string phase, object data) {
        if (publicationOverride != null) { publicationOverride(phase, data); return; }
        CdrCross.Atomic(PathFor(phase + "-" + role), CdrCross.Serialize(Packet(phase, data)), null);
    }
    private void Fail(string error) {
        Interlocked.CompareExchange(ref primary, error, null);
        if (Interlocked.Exchange(ref abortSent, 1) == 0) {
            try { Put("abort", CdrCross.D("error", primary)); }
            catch (Exception publication) { lock (cleanup) cleanup.Add("abort publication: " + CdrCross.ErrorText(publication)); }
        }
    }
    private void Check(bool requirePeer) {
        if (primary != null) throw new InvalidOperationException(primary);
        if (CdrCross.Now >= expiry) throw new TimeoutException("absolute diagnostic deadline");
        if (File.Exists(PathFor("abort-" + Other))) throw new InvalidOperationException("peer aborted; no replay");
        if (tap != null && !tap.Faults.IsEmpty) throw new InvalidOperationException("subscription fault");
        if (requirePeer && (peerHandle == null || peerHandle.Exited(0))) throw new InvalidOperationException("peer identity exited");
    }
    private Dictionary<string, object> Take(string phase, bool final) {
        string key = phase + "-" + Other;
        if (consumed.Contains(key)) throw new InvalidOperationException("duplicate phase consumption");
        string path = PathFor(key); if (!File.Exists(path)) return null;
        var value = CdrCross.Read(path);
        if (peer == null || !CdrCross.PeerEnvelope(value, nonce, bundle, Other, phase, expiry,
            (int)CdrCross.N(peer["pid"]), CdrCross.Text(peer, "created")) || CdrCross.N(value["frequency"]) != CdrCross.Frequency ||
            (!final && CdrCross.Now >= expiry)) throw new InvalidDataException("stale or unbound peer envelope");
        if (phase != "hello" && CdrCross.N(value["epoch"]) != epoch) throw new InvalidDataException("different acquisition window");
        consumed.Add(key); return CdrCross.Map(value["data"]);
    }
    private Dictionary<string, object> Wait(string phase, long deadline, bool final) {
        while (CdrCross.Now < deadline) {
            var value = Take(phase, final); if (value != null) return value;
            if (final) {
                if (peerHandle.Exited(0)) throw new InvalidOperationException("peer exited without final receipt");
            } else Check(true);
            Thread.Sleep(2);
        }
        throw new TimeoutException("missing phase " + phase);
    }
    private void Watchdog() {
        while (!finished.Wait(20)) {
            if (CdrCross.Now < expiry) continue;
            watchdogUsed = true;
            // Even a stuck cleanup cannot keep this diagnostic peer alive indefinitely.
            Fail("independent absolute self-deadline reached"); Cleanup();
            try { Put("expired", CdrCross.D("safe_for_follow_up", false, "cleanup_complete", cleaned.IsSet, "error", primary)); }
            catch { /* No successful receipt is claimed when publication is impossible. */ }
            Environment.Exit(2);
        }
    }
    private static string Quote(string value) {
        if (value.IndexOfAny(new char[] { '"', '\r', '\n' }) >= 0) throw new InvalidDataException("unquotable fixed path");
        return "\"" + value + "\"";
    }
    private void LaunchB() {
        string executable = script;
        string command = Quote(executable) + " peer " + Quote(directory) + " " + nonce + " " + bundle +
            " " + CdrCross.S(expiry) + " " + imagePin + " " + Quote(fixtures);
        long before = DateTime.UtcNow.ToFileTimeUtc();
        launchAttempted = true;
        using (var klass = new ManagementClass(@"\\.\root\cimv2:Win32_Process"))
        using (var startupClass = new ManagementClass(@"\\.\root\cimv2:Win32_ProcessStartup"))
        using (var startup = startupClass.CreateInstance())
        using (var input = klass.GetMethodParameters("Create")) {
            startup["ShowWindow"] = (ushort)0; // Default flags; no job breakaway or token adjustment.
            input["CommandLine"] = command; input["CurrentDirectory"] = Path.GetDirectoryName(script);
            input["ProcessStartupInformation"] = startup;
            using (var result = klass.InvokeMethod("Create", input, new InvokeMethodOptions { Timeout = TimeSpan.FromSeconds(15) })) {
                if (Convert.ToUInt32(result["ReturnValue"], CultureInfo.InvariantCulture) != 0)
                    throw new InvalidOperationException("WMI Create rejected; launch not retried");
                int child = Convert.ToInt32(result["ProcessId"], CultureInfo.InvariantCulture);
                peerHandle = new CdrCrossHandle(child); peer = peerHandle.Identity();
                if (CdrCross.N(peer["created"]) < before || peerHandle.Exited(0) ||
                    !String.Equals(CdrCross.Text(peer, "image"), executable, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidDataException("WMI returned PID identity is unproven");
            }
        }
    }
    private void BindA() {
        var hello = CdrCross.Read(PathFor("hello-A"));
        int otherPid = (int)CdrCross.N(hello["pid"]);
        peerHandle = new CdrCrossHandle(otherPid); peer = peerHandle.Identity();
        if (!CdrCross.PeerEnvelope(hello, nonce, bundle, "A", "hello", expiry, otherPid, CdrCross.Text(peer, "created")))
            throw new InvalidDataException("A self-handshake mismatch");
        consumed.Add("hello-A");
        long sampled = CdrCross.N(peer["sample_qpc"]);
        if (CdrCross.N(hello["frequency"]) != CdrCross.Frequency || CdrCross.N(hello["sent"]) > sampled)
            throw new InvalidDataException("QPC reference not comparable");
        if (peerHandle.Exited(0)) throw new InvalidOperationException("A exited before B initialization");
    }
    private Dictionary<string, object> Lineage() {
        var rows = new List<object>(); var seen = new HashSet<int>(); int current = pid; string limit = null;
        for (int i = 0; i < 16 && current > 0; i++) {
            if (!seen.Add(current)) { limit = "cycle"; break; }
            try {
                using (var handle = new CdrCrossHandle(current)) {
                    var value = handle.Identity(); rows.Add(CdrCross.D("pid", current, "created", value["created"], "image", value["image"]));
                    current = (int)CdrCross.N(value["parent_pid"]);
                }
            } catch (Exception error) { limit = error.Message; break; }
        }
        return CdrCross.D("rows", rows, "remaining_parent", current, "limit", limit);
    }
    private void VerifyPeers(Dictionary<string, object> hello) {
        Check(true);
        var reported = CdrCross.Map(hello["identity"]);
        if (CdrCross.N(reported["pid"]) != CdrCross.N(peer["pid"]) || CdrCross.Text(reported, "created") != CdrCross.Text(peer, "created") ||
            CdrCross.N(reported["frequency"]) != CdrCross.Frequency || CdrCross.N(reported["sample_qpc"]) > CdrCross.Now ||
            !CdrCross.TokenEqual(CdrCross.Map(peer["token"]), CdrCross.Map(self["token"])))
            throw new InvalidOperationException("CONTEXT_MISMATCH: peer identity/token/QPC");
        if (!CdrCross.EffectiveThread(CdrCross.Map(hello["thread"]), CdrCross.Map(peer["token"])))
            throw new InvalidOperationException("CONTEXT_MISMATCH: peer calling thread");
        var b = role == "B" ? self : peer;
        providerHandle = new CdrCrossHandle((int)CdrCross.N(b["parent_pid"]));
        var provider = providerHandle.Identity();
        string image = Path.GetFileName(CdrCross.Text(provider, "image"));
        if (providerHandle.Exited(0) || !String.Equals(image, "WmiPrvSE.exe", StringComparison.OrdinalIgnoreCase) ||
            CdrCross.N(provider["created"]) > CdrCross.N(b["created"]))
            throw new InvalidOperationException("CONTEXT_UNPROVEN: live provider parent");
        var aLineage = role == "A" ? Lineage() : CdrCross.Map(hello["lineage"]);
        foreach (var row in CdrCross.ArrayOf(aLineage["rows"])) {
            var identity = CdrCross.Map(row);
            if (CdrCross.N(identity["pid"]) == CdrCross.N(provider["pid"]) &&
                CdrCross.Text(identity, "created") == CdrCross.Text(provider, "created"))
                throw new InvalidOperationException("CONTEXT_UNPROVEN: provider is in A lineage");
        }
        self["provider_parent"] = provider; self["a_lineage"] = aLineage;
        self["contrast"] = "Measured live provider-parent lineage only; nested Job isolation UNPROVEN";
    }
    private Dictionary<string, object> Hello() {
        return CdrCross.D("identity", self, "thread", CdrRouteContext.Thread(), "lineage", Lineage());
    }
    private void StartWindow() {
        while (CdrCross.Now < epoch) { Check(true); Thread.Sleep(1); }
        Check(true);
        lock (owned) { tap = new CdrCrossTap(epoch, Fail); tap.Start(); }
        while (!tap.Ready && CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, 1000)) { Check(true); Thread.Sleep(2); }
        if (!tap.Ready) throw new InvalidOperationException("SETUP_UNPROVEN: first enumeration missing");
        var setup = CdrCross.D("first", tap.First.Select(CdrCross.S).ToArray(), "threads", tap.Contexts,
            "healthy", tap.Faults.IsEmpty, "identity", self, "window_utc", CdrCross.S(windowUtc));
        Put("setup", setup);
        var other = Wait("setup", epoch + CdrCross.Frequency, false);
        setupA = role == "A" ? setup : other; setupB = role == "B" ? setup : other;
        if (!SetupValid(CdrCross.Now)) throw new InvalidOperationException("SETUP_UNPROVEN: four-subscription gate");
    }
    private bool SetupValid(long dispatch) {
        if (setupA == null || setupB == null || tap == null || !tap.Ready || primary != null ||
            File.Exists(PathFor("abort-" + Other)) || providerHandle == null || providerHandle.Exited(0)) return false;
        var first = new List<long>();
        foreach (var setup in new Dictionary<string, object>[] { setupA, setupB }) {
            if (!(bool)setup["healthy"] || CdrCross.N(setup["window_utc"]) != windowUtc) return false;
            var id = CdrCross.Map(setup["identity"]);
            var expected = setup == setupA ? (role == "A" ? self : peer) : (role == "B" ? self : peer);
            if (CdrCross.N(id["pid"]) != CdrCross.N(expected["pid"]) || CdrCross.Text(id, "created") != CdrCross.Text(expected, "created")) return false;
            object[] contexts = CdrCross.ArrayOf(setup["threads"]), stamps = CdrCross.ArrayOf(setup["first"]);
            if (contexts.Length != 2 || stamps.Length != 2) return false;
            for (int i = 0; i < 2; i++) {
                if (!CdrCross.EffectiveThread(CdrCross.Map(contexts[i]), CdrCross.Map(expected["token"]))) return false;
                first.Add(CdrCross.N(stamps[i]));
            }
        }
        return CdrCross.Setup(first.ToArray(), new bool[] { true, true, true, true }, dispatch, epoch, CdrCross.Frequency);
    }
    private CdrCrossProbe NewProbe(string label, CdrCrossOnce gate, bool condition, int limit) {
        Check(true);
        lock (owned) {
            Check(true);
            bool timely = CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, limit);
            if (limit == 1000) timely &= SetupValid(CdrCross.Now);
            if (!gate.Take(condition && timely)) throw new InvalidOperationException("probe authorization closed");
            if (probes.Count >= (role == "A" ? 1 : 2)) throw new InvalidOperationException("controlled probe cap");
            var probe = new CdrCrossProbe(label, pid); probes.Add(probe);
            probe.Start(self, epoch, limit, delegate {
                Check(true);
                return condition && tap != null && tap.Ready && tap.Faults.IsEmpty &&
                    (limit != 1000 || SetupValid(CdrCross.Now));
            });
            return probe;
        }
    }
    private void AwaitReady(CdrCrossProbe probe, int limit) {
        while (CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, limit)) {
            Check(true); if (probe.Ready()) return; Thread.Sleep(1);
        }
        throw new TimeoutException("probe READY deadline");
    }
    private void AwaitExit(CdrCrossProbe probe, int limit) {
        while (CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, limit)) {
            Check(true);
            if (probe.ObserveExit()) {
                if (CdrCross.N(probe.Record["exit_code"]) != 0) throw new InvalidOperationException("probe exit code");
                return;
            }
            Thread.Sleep(1);
        }
        throw new TimeoutException("probe exit deadline");
    }
    public static bool Match(Dictionary<string, object> row, Dictionary<string, object> probe, string kind,
                             long epoch, long frequency, long windowUtc, int limit) {
        try {
            long time = CdrCross.N(row["created"]), received = CdrCross.N(row["received_qpc"]);
            return CdrCross.Text(row, "kind") == kind && CdrCross.N(row["pid"]) == CdrCross.N(probe["pid"]) &&
                CdrCross.N(row["parent"]) == CdrCross.N(probe["parent"]) &&
                String.Equals(CdrCross.Text(row, "name"), "cmd.exe", StringComparison.OrdinalIgnoreCase) &&
                time >= windowUtc && time >= CdrCross.N(probe["created"]) && time <= CdrCross.N(row["received_utc"]) &&
                received >= CdrCross.N(probe["dispatch"]) && CdrCross.Before(received, epoch, frequency, limit) &&
                (kind == "start" ? time <= CdrCross.N(probe["exited"]) : time >= CdrCross.N(probe["release_utc"]));
        } catch { return false; }
    }
    private bool Delivered(object[] rows, Dictionary<string, object> probe, string kind, int limit) {
        return rows.Any(value => Match(CdrCross.Map(value), probe, kind, epoch, CdrCross.Frequency, windowUtc, limit));
    }
    private Dictionary<string, object> Control(string label, CdrCrossOnce gate, bool condition, int limit) {
        var probe = NewProbe(label, gate, condition, limit);
        AwaitReady(probe, limit); probe.Release(); AwaitExit(probe, limit);
        while (CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, limit)) {
            Check(true); object[] rows = tap.Rows.ToArray().Cast<object>().ToArray();
            if (Delivered(rows, probe.Record, "start", limit) && Delivered(rows, probe.Record, "stop", limit))
                return CdrCross.D("probe", probe.Record, "rows", rows);
            Thread.Sleep(2);
        }
        throw new InvalidOperationException("CONTROL_DELIVERY_FAILED: " + label);
    }
    private bool ExitProof(Dictionary<string, object> probe, int expectedParent) {
        if (!(bool)probe["ready"] || !(bool)probe["exit_confirmed"] || (bool)probe["forced"] ||
            !(bool)probe["token_match"] || CdrCross.N(probe["exit_code"]) != 0 || CdrCross.N(probe["parent"]) != expectedParent ||
            CdrCross.N(probe["exited"]) <= CdrCross.N(probe["created"])) return false;
        using (var handle = new CdrCrossHandle((int)CdrCross.N(probe["pid"]))) {
            long[] times = handle.Times();
            return handle.Exited(0) && handle.ExitCode() == 0 && times[0] == CdrCross.N(probe["created"]) && times[1] == CdrCross.N(probe["exited"]);
        }
    }
    private void RunA() {
        Put("hello", Hello()); long sent = CdrCross.Now; LaunchB();
        var hello = Wait("hello", expiry, false);
        if (CdrCross.N(CdrCross.Map(hello["identity"])["sample_qpc"]) < sent) throw new InvalidDataException("B QPC precedes launch");
        VerifyPeers(hello); launchConfirmed = true;
        epoch = CdrCross.Now + CdrCross.Frequency / 2;
        windowUtc = DateTime.UtcNow.ToFileTimeUtc() + 5000000;
        Put("window", CdrCross.D("epoch", CdrCross.S(epoch), "window_utc", CdrCross.S(windowUtc), "frequency", CdrCross.Frequency));
        StartWindow(); pre = Wait("pre", epoch + CdrCross.Frequency, false);
        var preProbe = CdrCross.Map(pre["probe"]); var rows = CdrCross.ArrayOf(pre["rows"]);
        bool proof = CdrCross.AllowHeld(SetupValid(CdrCross.N(preProbe["dispatch"])) && SetupValid(CdrCross.Now),
            Delivered(rows, preProbe, "start", 1000), Delivered(rows, preProbe, "stop", 1000),
            ExitProof(preProbe, (int)CdrCross.N(peer["pid"])), CdrCross.Now, epoch, CdrCross.Frequency);
        var probe = NewProbe("A-held", heldGate, proof, 1000); AwaitReady(probe, 1000);
        while (CdrCross.Now - epoch < CdrCross.Frequency * 5 / 2) { Check(true); Thread.Sleep(1); }
        Check(true); probe.Release(); AwaitExit(probe, 5000); held = probe.Record; Put("held", held);
        post = Wait("post", epoch + CdrCross.Frequency * 5, false);
        while (CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, 5000)) { Check(true); Thread.Sleep(2); }
    }
    private void RunB() {
        BindA(); var aHello = CdrCross.Map(CdrCross.Read(PathFor("hello-A"))["data"]);
        VerifyPeers(aHello); Put("hello", Hello());
        // Window is the only message that establishes the epoch; all later messages must match it.
        while (!File.Exists(PathFor("window-A"))) { Check(true); Thread.Sleep(2); }
        var windowPacket = CdrCross.Read(PathFor("window-A"));
        if (!CdrCross.PeerEnvelope(windowPacket, nonce, bundle, "A", "window", expiry, (int)CdrCross.N(peer["pid"]), CdrCross.Text(peer, "created")))
            throw new InvalidDataException("window envelope mismatch");
        consumed.Add("window-A"); var window = CdrCross.Map(windowPacket["data"]);
        epoch = CdrCross.N(window["epoch"]); windowUtc = CdrCross.N(window["window_utc"]);
        if (CdrCross.N(windowPacket["epoch"]) != epoch || CdrCross.N(window["frequency"]) != CdrCross.Frequency ||
            epoch < CdrCross.N(windowPacket["sent"]) || epoch + CdrCross.Frequency * 5 >= expiry)
            throw new InvalidDataException("invalid shared window");
        StartWindow(); pre = Control("B-pre", preGate, SetupValid(CdrCross.Now), 1000); Put("pre", pre);
        held = Wait("held", epoch + CdrCross.Frequency * 5, false);
        bool proof = CdrCross.AllowPost((bool)held["ready"], ExitProof(held, (int)CdrCross.N(peer["pid"])),
            CdrCross.N(held["release"]), CdrCross.Now, epoch, CdrCross.Frequency);
        post = Control("B-post", postGate, proof, 5000); Put("post", post);
        while (CdrCross.Before(CdrCross.Now, epoch, CdrCross.Frequency, 5000)) { Check(true); Thread.Sleep(2); }
    }
    private void Cleanup() {
        if (Interlocked.CompareExchange(ref cleanupClaim, 1, 0) != 0) { cleaned.Wait(3500); return; }
        try {
            lock (owned) {
                foreach (var probe in probes) probe.Cleanup(cleanup);
                if (tap != null) CdrCross.Attempt(cleanup, "tap.dispose", tap.Dispose);
            }
        } catch (Exception error) { cleanup.Add("cleanup dispatcher: " + CdrCross.ErrorText(error)); }
        finally { cleaned.Set(); }
    }
    private bool LocalSafe() {
        return cleaned.IsSet && !watchdogUsed && watchdogJoined && cleanup.Count == 0 &&
            (tap == null || (tap.Joined && tap.CleanupErrors.Count == 0)) &&
            probes.All(value => value.Started && value.ExitConfirmed && !value.Forced);
    }
    private Dictionary<string, object> LocalReport() {
        return CdrCross.D("diagnostic_only", true, "native_gate_pass", false, "role", role, "identity", self,
            "epoch", CdrCross.S(epoch), "frequency", CdrCross.Frequency, "window_utc", CdrCross.S(windowUtc), "budget_ms", 5000,
            "probe_cap", 3, "setup_a", setupA, "setup_b", setupB, "observer", tap == null ? null : tap.Report(),
            "probes", probes.Select(value => value.Record).ToArray(), "error", primary, "cleanup_errors", cleanup.ToArray(),
            "cleanup_complete", cleaned.IsSet, "safe_for_follow_up", LocalSafe(), "self_deadline_used", watchdogUsed);
    }
    public static Dictionary<string, object> Matrix(object[] aRows, object[] bRows,
            Dictionary<string, object> held, Dictionary<string, object> preProbe, Dictionary<string, object> postProbe,
            long epoch, long frequency, long windowUtc) {
        Func<object[], Dictionary<string, object>, string, int, bool> delivered = delegate(object[] rows, Dictionary<string, object> probe, string kind, int limit) {
            return rows.Any(value => Match(CdrCross.Map(value), probe, kind, epoch, frequency, windowUtc, limit));
        };
        var outcomes = new Dictionary<string, object>();
        foreach (string kind in new string[] { "start", "stop" }) {
            bool aa = delivered(aRows, held, kind, 5000), ba = delivered(bRows, held, kind, 5000);
            // B's early authorization remains 1s. A's final observation uses the entire 5s window.
            bool ab = delivered(aRows, preProbe, kind, 5000) && delivered(aRows, postProbe, kind, 5000);
            bool bb = delivered(bRows, preProbe, kind, 1000) && delivered(bRows, postProbe, kind, 5000);
            if (!bb) return new Dictionary<string, object>();
            outcomes[kind] = CdrCross.D("a_sees_a", aa, "a_sees_b_both", ab, "b_sees_a", ba, "b_sees_b_both", bb,
                "interpretation", CdrCross.Cause(aa, ab, ba, bb));
        }
        return outcomes;
    }
    private Dictionary<string, object> Comparison(Dictionary<string, object> local) {
        bool safe = LocalSafe() && launchConfirmed && peerExitConfirmed && finalB != null && (bool)finalB["safe_for_follow_up"];
        bool valid = safe && primary == null && finalB["error"] == null && pre != null && held != null && post != null;
        object[] aRows = tap == null ? new object[0] : tap.Rows.ToArray().Cast<object>().ToArray();
        object[] bRows = finalB == null || finalB["observer"] == null ? new object[0] : CdrCross.ArrayOf(CdrCross.Map(finalB["observer"])["rows"]);
        var outcomes = new Dictionary<string, object>();
        if (valid) {
            var preProbe = CdrCross.Map(pre["probe"]); var postProbe = CdrCross.Map(post["probe"]);
            valid = tap.Faults.IsEmpty && CdrCross.ArrayOf(CdrCross.Map(finalB["observer"])["faults"]).Length == 0;
            outcomes = Matrix(aRows, bRows, held, preProbe, postProbe, epoch, CdrCross.Frequency, windowUtc);
            valid &= outcomes.Count == 2;
        }
        if (!valid) outcomes.Clear();
        return CdrCross.D("diagnostic_only", true, "native_gate_pass", false, "comparison_valid", valid,
            "safe_for_follow_up", safe, "status", valid ? "COMPARISON_RECORDED" : "INCONCLUSIVE",
            "scope", "Read-only launch-lineage/observer comparison, not Job causation or native QA PASS",
            "a", local, "b", finalB, "outcomes", outcomes, "b_launch_attempted", launchAttempted,
            "b_launch_confirmed", launchConfirmed, "b_exit_confirmed", peerExitConfirmed, "run_directory", directory);
    }
    private Dictionary<string, object> PublishFinalReport() {
        var local = LocalReport(); var result = role == "A" ? Comparison(local) : local;
        try { Put("final", result); }
        catch (Exception error) {
            Fail("final publication: " + CdrCross.ErrorText(error));
            // Fail may append abort-publication errors after LocalReport took its snapshot.
            local["error"] = primary;
            lock (cleanup) local["cleanup_errors"] = cleanup.ToArray();
            local["safe_for_follow_up"] = false;
            CdrCross.PublicationFailure(result, error);
        }
        return result;
    }
    public static Dictionary<string, object> FinalizationContract(string directory, string role, bool priorFailure) {
        var attempts = new List<string>();
        string suffix = role + (priorFailure ? "-prior" : "-fresh");
        string finalPath = Path.Combine(directory, "cascade-final-" + suffix + ".json");
        string abortPath = Path.Combine(directory, "cascade-abort-" + suffix + ".json");
        Action<string, object> publisher = delegate(string phase, object data) {
            attempts.Add(phase);
            if (phase != "final" && phase != "abort") throw new InvalidOperationException("unexpected contract publication phase");
            try { CdrCross.Atomic(phase == "final" ? finalPath : abortPath, CdrCross.Serialize(data), "both"); }
            catch (Exception original) {
                var marked = new IOException(phase + " " + original.Message, original);
                if (original.Data.Contains("secondary_close_error"))
                    marked.Data["secondary_close_error"] = phase + " " + original.Data["secondary_close_error"];
                throw marked;
            }
        };
        var session = new CdrCrossSession(role, directory, publisher);
        try {
            if (priorFailure) session.Fail("original earlier failure");
            var result = session.PublishFinalReport();
            string serialized = CdrCross.Serialize(result);
            var output = CdrCross.Json().Deserialize<Dictionary<string, object>>(serialized);
            var local = role == "A" ? CdrCross.Map(output["a"]) : output;
            string primary = CdrCross.Text(local, "error");
            var finalError = CdrCross.Map(output["publication_error"]);
            object[] cleanup = CdrCross.ArrayOf(local["cleanup_errors"]);
            bool errorsPresent = CdrCross.Text(finalError, "message") == "final injected write failure" &&
                CdrCross.Text(finalError, "secondary_close_error") == "final injected close failure" &&
                cleanup.Length == 1 && Convert.ToString(cleanup[0], CultureInfo.InvariantCulture).Contains("abort injected write failure") &&
                Convert.ToString(cleanup[0], CultureInfo.InvariantCulture).Contains("abort injected close failure");
            bool priority = priorFailure ? primary == "original earlier failure" :
                primary.Contains("final injected write failure") && primary.Contains("final injected close failure");
            bool safe = !(bool)output["safe_for_follow_up"] && !(bool)local["safe_for_follow_up"] &&
                !(bool)output["native_gate_pass"] && !(bool)local["native_gate_pass"];
            bool count = String.Join(",", attempts.ToArray()) == (priorFailure ? "abort,final" : "final,abort");
            bool unpublished = !File.Exists(finalPath) && !File.Exists(abortPath);
            return CdrCross.D("pass", errorsPresent && priority && safe && count && unpublished,
                "role", role, "prior_failure", priorFailure, "attempts", attempts.ToArray(),
                "errors_present", errorsPresent, "first_error_preserved", priority, "unsafe_preserved", safe,
                "files_unpublished", unpublished, "serialized_output", serialized);
        } finally { session.cleaned.Dispose(); session.finished.Dispose(); }
    }
    public Dictionary<string, object> Execute() {
        try { if (role == "A") RunA(); else RunB(); }
        catch (Exception error) { Fail(CdrCross.ErrorText(error)); }
        finally { Cleanup(); }
        if (role == "A" && peer != null) {
            try {
                finalB = Wait("final", expiry, true);
                peerExitConfirmed = peerHandle.Exited(1000) && peerHandle.ExitCode() == 0;
                if (!peerExitConfirmed) Fail("B final receipt has no clean pinned exit");
            } catch (Exception error) { Fail("B finalization: " + CdrCross.ErrorText(error)); }
        }
        foreach (var handle in new CdrCrossHandle[] { providerHandle, peerHandle, selfHandle }) {
            if (handle != null) CdrCross.Attempt(cleanup, "identity.handle", handle.Dispose);
        }
        finished.Set(); watchdogJoined = watchdog.Join(1000);
        if (!watchdogJoined) cleanup.Add("independent watchdog Join unconfirmed");
        return PublishFinalReport();
    }
}

public static class CdrCrossContracts {
    private static bool DispatchCase(long stamp, int limit, bool permission, bool delay, bool repeat) {
        int starts = 0; long clock = delay ? stamp - 1 : stamp;
        var probe = new CdrCrossProbe("contract-only", 0);
        Func<bool> permitted = delegate { if (delay) clock = stamp; return permission; };
        bool allowed = permission && stamp < limit;
        try { probe.Dispatch(delegate { return clock; }, 0, 1000, limit, permitted, delegate { starts++; return true; }); }
        catch (InvalidOperationException) { if (allowed) return false; }
        if (repeat) {
            try { probe.Dispatch(delegate { return 0; }, 0, 1000, limit, delegate { return true; }, delegate { starts++; return true; }); }
            catch (InvalidOperationException) { }
        }
        probe.Process.Dispose();
        return starts == (allowed ? 1 : 0) && CdrCross.N(probe.Record["dispatch"]) == stamp;
    }
    private static object Event(Dictionary<string, object> probe, string kind, int stamp) {
        return CdrCross.D("kind", kind, "pid", probe["pid"], "parent", probe["parent"], "name", "cmd.exe",
            "created", CdrCross.S(CdrCross.N(probe[kind == "start" ? "created" : "release_utc"]) + 1),
            "received_utc", "1000", "received_qpc", CdrCross.S(stamp));
    }
    private static Dictionary<string, object> MatrixCase(int aPre, int bPre) {
        var pre = CdrCross.D("pid", 71, "parent", 7, "created", "100", "exited", "140", "dispatch", "200", "release_utc", "120");
        var held = CdrCross.D("pid", 72, "parent", 8, "created", "150", "exited", "190", "dispatch", "800", "release_utc", "180");
        var post = CdrCross.D("pid", 73, "parent", 7, "created", "200", "exited", "240", "dispatch", "2600", "release_utc", "220");
        object[] a = { Event(pre, "start", aPre), Event(pre, "stop", aPre + 100), Event(post, "start", 3000), Event(post, "stop", 3100) };
        object[] b = { Event(pre, "start", bPre), Event(pre, "stop", bPre + 100), Event(held, "start", 900), Event(held, "stop", 2800),
            Event(post, "start", 3000), Event(post, "stop", 3100) };
        return CdrCrossSession.Matrix(a, b, held, pre, post, 0, 1000, 90);
    }
    private static Dictionary<string, object> Row(string name, bool pass) { return CdrCross.D("name", name, "pass", pass); }
    public static Dictionary<string, object> Run(string directory, bool legacy) {
        var cases = new List<object>();
        long[] normal = { 100, 110, 120, 130 }; bool[] healthy = { true, true, true, true };
        // Design counterexample only: the old written plan lacked this rule. No OS probes.
        long[] lateA = { 900, 110, 120, 130 };
        bool actual = legacy ? CdrCross.Before(300, 0, 1000, 1000) : CdrCross.Setup(lateA, healthy, 200, 0, 1000);
        cases.Add(Row("late_a_setup_blocks_pre_control", !actual));
        if (legacy) return CdrCross.D("diagnostic_only", true, "native_gate_pass", false, "evidence_kind", "synthetic_previous_design_counterexample", "contracts", cases);
        cases.Add(Row("four_ready_then_control", CdrCross.Setup(normal, healthy, 200, 0, 1000)));
        bool missing = true, late = true, faults = true;
        for (int i = 0; i < 4; i++) {
            var times = (long[])normal.Clone(); times[i] = -1; missing &= !CdrCross.Setup(times, healthy, 200, 0, 1000);
            times[i] = 201; late &= !CdrCross.Setup(times, healthy, 200, 0, 1000);
            var states = (bool[])healthy.Clone(); states[i] = false; faults &= !CdrCross.Setup(normal, states, 200, 0, 1000);
        }
        cases.Add(Row("each_missing_subscription", missing));
        cases.Add(Row("each_late_subscription", late));
        cases.Add(Row("each_faulted_subscription", faults));
        cases.Add(Row("deadline_999", CdrCross.Setup(normal, healthy, 999, 0, 1000)));
        cases.Add(Row("deadline_1000", !CdrCross.Setup(normal, healthy, 1000, 0, 1000)));
        cases.Add(Row("deadline_1001", !CdrCross.Setup(normal, healthy, 1001, 0, 1000)));
        cases.Add(Row("same_tick_is_not_preceding", !CdrCross.Setup(normal, healthy, 130, 0, 1000)));
        cases.Add(Row("negative_window_rejected", !CdrCross.Before(-1, 0, 1000, 1000)));
        cases.Add(Row("only_empty_poll_is_setup", CdrCrossTap.EmptyPoll(ManagementStatus.Timedout) &&
            !CdrCrossTap.EmptyPoll(ManagementStatus.AccessDenied) && !CdrCrossTap.EmptyPoll(ManagementStatus.Failed)));
        var once = new CdrCrossOnce(); bool first = once.Take(true);
        cases.Add(Row("authorization_consumed_once", first && !once.Take(true)));
        cases.Add(Row("invalid_control_does_not_authorize", !new CdrCrossOnce().Take(false)));
        bool evidence = CdrCross.AllowHeld(true, true, true, true, 300, 0, 1000);
        cases.Add(Row("start_and_stop_and_exit_required", evidence && !CdrCross.AllowHeld(true, false, true, true, 300, 0, 1000) &&
            !CdrCross.AllowHeld(true, true, false, true, 300, 0, 1000) && !CdrCross.AllowHeld(true, true, true, false, 300, 0, 1000)));
        cases.Add(Row("late_control_does_not_reopen", !CdrCross.AllowHeld(true, true, true, true, 1000, 0, 1000)));
        cases.Add(Row("late_fault_invalidates_without_replay", !CdrCross.AllowHeld(false, true, true, true, 300, 0, 1000) && !once.Take(true)));
        cases.Add(Row("post_requires_ready_and_exact_exit", CdrCross.AllowPost(true, true, 2500, 3000, 0, 1000) &&
            !CdrCross.AllowPost(false, true, 2500, 3000, 0, 1000) && !CdrCross.AllowPost(true, false, 2500, 3000, 0, 1000)));
        cases.Add(Row("post_stays_inside_window", !CdrCross.AllowPost(true, true, 2500, 5000, 0, 1000) &&
            !CdrCross.AllowPost(true, true, 2499, 3000, 0, 1000)));
        var packet = CdrCross.D("nonce", "n", "bundle", "b", "role", "B", "phase", "pre", "expiry", "6000", "pid", 7, "created", "123", "sent", "200");
        cases.Add(Row("exact_envelope", CdrCross.PeerEnvelope(packet, "n", "b", "B", "pre", 6000, 7, "123")));
        cases.Add(Row("stale_nonce_rejected", !CdrCross.PeerEnvelope(packet, "old", "b", "B", "pre", 6000, 7, "123")));
        cases.Add(Row("wrong_identity_rejected", !CdrCross.PeerEnvelope(packet, "n", "b", "B", "pre", 6000, 8, "123") &&
            !CdrCross.PeerEnvelope(packet, "n", "b", "B", "pre", 6000, 7, "124")));
        cases.Add(Row("wrong_phase_or_expiry_rejected", !CdrCross.PeerEnvelope(packet, "n", "b", "B", "post", 6000, 7, "123") &&
            !CdrCross.PeerEnvelope(packet, "n", "b", "B", "pre", 6001, 7, "123")));
        var probe = CdrCross.D("pid", 71, "parent", 7, "created", "100", "exited", "140", "dispatch", "200", "release_utc", "120");
        var row = CdrCross.D("kind", "start", "pid", 71, "parent", 7, "name", "cmd.exe", "created", "110", "received_utc", "150", "received_qpc", "250");
        cases.Add(Row("exact_row_matches", CdrCrossSession.Match(row, probe, "start", 0, 1000, 90, 1000)));
        row["received_qpc"] = "1000";
        cases.Add(Row("late_row_rejected", !CdrCrossSession.Match(row, probe, "start", 0, 1000, 90, 1000)));
        row["received_qpc"] = "250"; row["pid"] = 72;
        cases.Add(Row("different_pid_rejected", !CdrCrossSession.Match(row, probe, "start", 0, 1000, 90, 1000)));
        row["pid"] = 71; row["created"] = "99";
        cases.Add(Row("old_process_row_rejected", !CdrCrossSession.Match(row, probe, "start", 0, 1000, 90, 1000)));
        foreach (string failure in new string[] { "partial_start", "join_throw", "join_false" }) {
            var tap = CdrCrossTap.ForContract(failure); string primary = "original";
            try { tap.Start(); } catch (Exception error) { primary = error.Message; }
            tap.Dispose(); bool reaped = tap.ReapContractThreads();
            bool expectedJoined = failure == "partial_start";
            cases.Add(Row(failure, reaped && tap.Joined == expectedJoined && tap.JoinAttempted[0] &&
                tap.JoinAttempted[1] == !expectedJoined && (expectedJoined ? primary.Contains("injected") : primary == "original")));
        }
        var errors = new List<string>(); var stages = new List<string>();
        foreach (string step in new string[] { "stdin", "wait", "kill", "final_wait", "dispose" }) {
            string stage = step;
            CdrCross.Attempt(errors, stage, delegate { stages.Add(stage); if (stage == "stdin" || stage == "kill") throw new IOException(stage); });
        }
        cases.Add(Row("cleanup_continues_after_stdin_and_kill_faults", errors.Count == 2 && String.Join(",", stages.ToArray()) == "stdin,wait,kill,final_wait,dispose"));
        foreach (string failure in new string[] { "write", "close", "both" }) {
            string destination = Path.Combine(directory, "publish-" + failure + ".json"); string error = null;
            try { CdrCross.Atomic(destination, "{}", failure); } catch (Exception caught) { error = caught.Message; }
            bool firstPreserved = failure != "both" || (error != null && error.Contains("write"));
            cases.Add(Row("publication_" + failure, !File.Exists(destination) && error != null && firstPreserved));
        }
        string committed = Path.Combine(directory, "committed.json"); CdrCross.Atomic(committed, "{\"original\":true}", null);
        bool conflict = false; try { CdrCross.Atomic(committed, "{}", null); } catch (IOException) { conflict = true; }
        cases.Add(Row("publication_is_no_clobber", conflict && File.ReadAllText(committed) == "{\"original\":true}"));
        cases.Add(Row("all_zero_is_inconclusive", CdrCross.Cause(false, false, false, false) == "INCONCLUSIVE_CONTROL"));
        cases.Add(Row("separate_context_outcomes", CdrCross.Cause(false, false, true, true) == "A_OBSERVER_CONTEXT_PATH" &&
            CdrCross.Cause(false, true, true, true) == "OBSERVER_PRODUCER_RELATION" &&
            CdrCross.Cause(false, true, false, true) == "A_PRODUCER_OR_COMMON_UPSTREAM"));
        cases.Add(Row("dispatch_999_once", DispatchCase(999, 1000, true, false, true)));
        cases.Add(Row("dispatch_1000_denied", DispatchCase(1000, 1000, true, false, true)));
        cases.Add(Row("dispatch_1001_denied", DispatchCase(1001, 1000, true, false, true)));
        cases.Add(Row("dispatch_post_5000_denied", DispatchCase(5000, 5000, true, false, true)));
        cases.Add(Row("dispatch_permission_check_delay", DispatchCase(1000, 1000, true, true, true)));
        cases.Add(Row("dispatch_fresh_fault_denied", DispatchCase(999, 1000, false, false, true)));
        var matrix = MatrixCase(1100, 300);
        cases.Add(Row("final_matrix_uses_full_a_window", matrix.Count == 2 && matrix.Values.All(value =>
            CdrCross.Text(CdrCross.Map(value), "interpretation") == "OBSERVER_PRODUCER_RELATION")));
        cases.Add(Row("final_matrix_keeps_b_early_gate", MatrixCase(1100, 1100).Count == 0));
        var outside = MatrixCase(5000, 300);
        cases.Add(Row("final_matrix_rejects_a_after_window", outside.Count == 2 && outside.Values.All(value =>
            !(bool)CdrCross.Map(value)["a_sees_b_both"])));
        var publication = CdrCross.D("safe_for_follow_up", true);
        string publicationPath = Path.Combine(directory, "final-output-both.json");
        try { CdrCross.Atomic(publicationPath, "{}", "both"); }
        catch (Exception error) { CdrCross.PublicationFailure(publication, error); }
        var serialized = CdrCross.Json().Deserialize<Dictionary<string, object>>(CdrCross.Serialize(publication));
        var detail = CdrCross.Map(serialized["publication_error"]);
        cases.Add(Row("final_output_retains_write_and_close", !(bool)serialized["safe_for_follow_up"] && !File.Exists(publicationPath) &&
            CdrCross.Text(detail, "message").Contains("write") && CdrCross.Text(detail, "secondary_close_error").Contains("close")));
        var cleanupOutput = new List<string>();
        CdrCross.Attempt(cleanupOutput, "publication", delegate { CdrCross.Atomic(Path.Combine(directory, "cleanup-both.json"), "{}", "both"); });
        cases.Add(Row("cleanup_output_retains_write_and_close", cleanupOutput.Count == 1 &&
            cleanupOutput[0].Contains("write") && cleanupOutput[0].Contains("secondary_close_error") && cleanupOutput[0].Contains("close")));
        foreach (string role in new string[] { "A", "B" }) {
            foreach (bool prior in new bool[] { false, true }) {
                var cascade = CdrCrossSession.FinalizationContract(directory, role, prior);
                cases.Add(CdrCross.D("name", "finalization_" + role + (prior ? "_prior" : "_fresh"),
                    "pass", cascade["pass"], "evidence", cascade));
            }
        }
        return CdrCross.D("diagnostic_only", true, "native_gate_pass", false, "contracts", cases,
            "limitations", "Deterministic helpers and managed-thread cleanup; not WMI launch/token/OS delivery proof");
    }
}

public static class CdrCrossEntry {
    // This method intentionally has no WMI, hashing, file I/O, or session construction.
    // Keep the guarded body out of this JIT unit so its dependencies load after the guard.
    public static int Main(string[] args) {
        long expiry;
        if (args.Length != 7 || !Int64.TryParse(args[4], NumberStyles.None, CultureInfo.InvariantCulture, out expiry)) return 2;
        long now = Stopwatch.GetTimestamp(), frequency = Stopwatch.Frequency;
        if (expiry <= now || expiry - now > frequency * 65) return 2;
        if (args[0] == "bootstrap-stall") expiry = Math.Min(expiry, now + frequency / 10);
        long remaining = (long)((decimal)(expiry - now) * 1000 / frequency);
        using (var hardDeadline = new Timer(delegate(object state) { Environment.Exit(3); },
                null, Math.Max(1, remaining + 5000), Timeout.Infinite)) {
            if (args[0] == "bootstrap-stall") Thread.Sleep(Timeout.Infinite);
            return GuardedRun(args, expiry);
        }
    }
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    private static int GuardedRun(string[] args, long expiry) {
        try {
            string mode = args[0], directory = args[1], nonce = args[2], bundle = args[3], imagePin = args[5], fixtures = args[6];
            if (mode != "capture" && mode != "peer" && mode != "contracts") throw new InvalidDataException("unknown fixed entry mode");
            if (nonce.Length != 32 || nonce.Any(value => !Uri.IsHexDigit(value)) ||
                bundle.Length != 64 || imagePin.Length != 64 || !Path.IsPathRooted(fixtures) ||
                !Path.IsPathRooted(directory) || Path.GetFileName(directory) != "cdr-cross-" + nonce)
                throw new InvalidDataException("invalid bootstrap identity");
            string image = System.Reflection.Assembly.GetExecutingAssembly().Location;
            if (!String.Equals(image, Path.Combine(directory, "native_process_cross_context.exe"), StringComparison.OrdinalIgnoreCase) ||
                (File.GetAttributes(directory) & FileAttributes.ReparsePoint) != 0)
                throw new InvalidDataException("unbound bootstrap image");
            // Retain a read-only, no-write/no-delete share through both runtime and final output.
            using (var lockedImage = new FileStream(image, FileMode.Open, FileAccess.Read, FileShare.Read)) {
                if (Hash(lockedImage) != imagePin || Bundle(fixtures) != bundle || CdrCross.Now >= expiry)
                    throw new InvalidDataException("expired or mismatched compiled bundle");
                Dictionary<string, object> result;
                if (mode == "contracts") result = CdrCrossContracts.Run(directory, false);
                else result = new CdrCrossSession(mode == "peer" ? "B" : "A",
                    directory, nonce, bundle, expiry, image, imagePin, fixtures).Execute();
                if (mode != "peer") { Console.Out.WriteLine(CdrCross.Serialize(result)); Console.Out.Flush(); }
                if (mode == "contracts") return CdrCross.ArrayOf(result["contracts"]).All(value => (bool)CdrCross.Map(value)["pass"]) ? 0 : 1;
                return (bool)result["safe_for_follow_up"] ? 0 : 1;
            }
        } catch (Exception error) {
            if (args[0] != "peer") {
                Console.Out.WriteLine(CdrCross.Serialize(CdrCross.D("diagnostic_only", true, "native_gate_pass", false,
                    "safe_for_follow_up", false, "bootstrap_error", CdrCross.ErrorEvidence(error))));
                Console.Out.Flush();
            }
            return 1;
        }
    }
    private static string Hash(Stream stream) {
        using (var hash = System.Security.Cryptography.SHA256.Create())
            return BitConverter.ToString(hash.ComputeHash(stream)).Replace("-", "").ToLowerInvariant();
    }
    private static string Bundle(string fixtures) {
        var lines = new List<string>();
        foreach (string name in new string[] { "native_process_cross_context.ps1", "native_process_cross_context.cs", "native_process_route_matrix.cs" }) {
            using (var stream = new FileStream(Path.Combine(fixtures, name), FileMode.Open, FileAccess.Read, FileShare.Read))
                lines.Add(name + ":" + Hash(stream));
        }
        using (var stream = new MemoryStream(new UTF8Encoding(false).GetBytes(String.Join("\n", lines.ToArray())))) return Hash(stream);
    }
}
