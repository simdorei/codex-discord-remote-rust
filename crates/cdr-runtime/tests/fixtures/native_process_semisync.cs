using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Globalization;
using System.Management;
using System.Diagnostics;
using System.Threading;

// Diagnostic comparison only. This is not a product readiness replacement.
public sealed class CdrSemisyncTap : IDisposable {
    public readonly ConcurrentQueue<CdrRouteRow> Rows = new ConcurrentQueue<CdrRouteRow>();
    public readonly ConcurrentQueue<string> Faults = new ConcurrentQueue<string>();
    public readonly ConcurrentQueue<string> CleanupFaults = new ConcurrentQueue<string>();
    public readonly object[] ThreadContexts = new object[2];
    public readonly int[] PollTimeouts = new int[2];
    private readonly Stopwatch clock;
    private readonly Thread[] threads = new Thread[2];
    private readonly ManualResetEventSlim[] ready = {
        new ManualResetEventSlim(false), new ManualResetEventSlim(false)
    };
    private int closing;
    private int count;
    private readonly Action<int> listener;
    private readonly Action<int, Thread> starter;
    private readonly Func<int, Thread, bool> joiner;
    private readonly bool synthetic;
    public bool ThreadsJoined;
    public readonly bool[] Started = new bool[2];
    public readonly bool[] JoinAttempted = new bool[2];
    public readonly long[] FirstEnumerationMs = { -1, -1 };

    public CdrSemisyncTap(Stopwatch clock) : this(clock, null, null, null) { }
    private CdrSemisyncTap(Stopwatch clock, Action<int> listener,
                          Action<int, Thread> starter, Func<int, Thread, bool> joiner) {
        this.clock = clock;
        this.synthetic = listener != null;
        this.listener = listener ?? Listen;
        this.starter = starter ?? delegate(int index, Thread thread) { thread.Start(); };
        this.joiner = joiner ?? delegate(int index, Thread thread) { return thread.Join(1000); };
    }
    private void EnumerationCompleted(int index) {
        Interlocked.CompareExchange(ref FirstEnumerationMs[index], clock.ElapsedMilliseconds, -1);
        ready[index].Set();
    }
    public static bool IsPollTimeout(int status) {
        return status == (int)ManagementStatus.Timedout;
    }
    private void Cleanup(string name, Action action) {
        try { action(); }
        catch (Exception error) { CleanupFaults.Enqueue(name + ": " + error.Message); }
    }
    private void Listen(int index) {
        ManagementEventWatcher watcher = null;
        try {
            ThreadContexts[index] = CdrRouteContext.Thread();
            string kind = index == 0 ? "start" : "stop";
            string className = index == 0 ? "Win32_ProcessStartTrace" : "Win32_ProcessStopTrace";
            var options = new EventWatcherOptions();
            options.Timeout = TimeSpan.FromMilliseconds(100);
            watcher = new ManagementEventWatcher(new ManagementScope(@"\\.\root\cimv2"),
                new WqlEventQuery("SELECT * FROM " + className), options);
            while (Volatile.Read(ref closing) == 0) {
                try {
                    // Do not call Start(): this intentionally uses the blocking enumerator path.
                    using (var value = watcher.WaitForNextEvent()) {
                        EnumerationCompleted(index);
                        if (Interlocked.Increment(ref count) > 4096)
                            throw new InvalidOperationException("semisynchronous raw row limit exceeded");
                        Rows.Enqueue(new CdrRouteRow {
                            Kind = kind,
                            ProcessId = Convert.ToUInt32(value["ProcessID"], CultureInfo.InvariantCulture),
                            ParentProcessId = Convert.ToUInt32(value["ParentProcessID"], CultureInfo.InvariantCulture),
                            Name = Convert.ToString(value["ProcessName"], CultureInfo.InvariantCulture),
                            CreatedTick = Convert.ToString(value["TIME_CREATED"], CultureInfo.InvariantCulture),
                            ReceivedUtcTick = DateTime.UtcNow.ToFileTimeUtc().ToString(CultureInfo.InvariantCulture),
                            ReceivedMs = clock.ElapsedMilliseconds
                        });
                    }
                } catch (ManagementException error) {
                    if (!IsPollTimeout((int)error.ErrorCode)) throw;
                    Interlocked.Increment(ref PollTimeouts[index]);
                    // A completed empty enumeration proves the call returned, not event delivery.
                    EnumerationCompleted(index);
                }
            }
        } catch (Exception error) {
            Faults.Enqueue(index + ": " + error.GetType().Name + ": " + error.Message);
        } finally {
            if (watcher != null) {
                Cleanup(index + ".Stop", delegate { watcher.Stop(); });
                Cleanup(index + ".Dispose", delegate { watcher.Dispose(); });
            }
        }
    }
    public void Start() {
        for (int index = 0; index < 2; index++) {
            int slot = index;
            threads[index] = new Thread(delegate() { listener(slot); });
            threads[index].IsBackground = true;
            starter(index, threads[index]);
            Started[index] = true;
        }
    }
    // No WMI subscription is created in these deterministic lifecycle contracts.
    public static CdrSemisyncTap ForCleanupContract(string fault) {
        if (fault != "partial_start" && fault != "join_throw" && fault != "join_false")
            throw new ArgumentException("Unknown cleanup contract");
        CdrSemisyncTap tap = null;
        tap = new CdrSemisyncTap(Stopwatch.StartNew(),
            delegate(int index) {
                while (Volatile.Read(ref tap.closing) == 0) Thread.Sleep(1);
            },
            delegate(int index, Thread thread) {
                if (fault == "partial_start" && index == 1)
                    throw new InvalidOperationException("injected second thread start failure");
                thread.Start();
            },
            delegate(int index, Thread thread) {
                if (index == 0 && fault == "join_throw")
                    throw new InvalidOperationException("injected join failure");
                if (index == 0 && fault == "join_false") return false;
                return thread.Join(1000);
            });
        return tap;
    }
    public bool ReapContractThreads() {
        if (!synthetic) throw new InvalidOperationException("Synthetic contract cleanup only");
        bool joined = true;
        foreach (var thread in threads) {
            if (thread != null && (thread.ThreadState & System.Threading.ThreadState.Unstarted) == 0)
                joined = thread.Join(1000) && joined;
        }
        if (joined && !ThreadsJoined) foreach (var signal in ready) signal.Dispose();
        return joined;
    }
    public bool Ready { get { return ready[0].IsSet && ready[1].IsSet && Faults.IsEmpty; } }
    public void Dispose() {
        Interlocked.Exchange(ref closing, 1);
        ThreadsJoined = true;
        for (int index = 0; index < 2; index++) {
            try {
                var thread = threads[index];
                if (thread == null || (thread.ThreadState & System.Threading.ThreadState.Unstarted) != 0)
                    continue;
                Started[index] = true;
                JoinAttempted[index] = true;
                if (!joiner(index, thread)) {
                    ThreadsJoined = false;
                    CleanupFaults.Enqueue(index + ": event enumeration thread did not exit");
                }
            } catch (Exception error) {
                ThreadsJoined = false;
                CleanupFaults.Enqueue(index + ".Join: " + error.Message);
            }
        }
        if (ThreadsJoined) foreach (var signal in ready)
            Cleanup("ready.Dispose", delegate { signal.Dispose(); });
    }
}

public sealed class CdrSemisyncGate {
    private bool consumed;
    public bool Authorize(bool ready, long nowMs) {
        if (consumed) return false;
        consumed = true;
        return ready && nowMs >= 0 && nowMs < 1000;
    }
}
