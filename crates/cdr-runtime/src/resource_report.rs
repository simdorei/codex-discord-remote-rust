//! Bounded, read-only host sampling for the resources command.
use std::time::Duration;
use tokio::sync::Semaphore;
#[cfg(windows)]
#[path = "resource_cpu.rs"]
mod cpu;

static PROBE_SLOT: Semaphore = Semaphore::const_new(1);
#[cfg(test)]
pub(crate) static TEST_HOST_PROBE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn bounded_probe<T: Send + 'static>(
    slot: &'static Semaphore,
    timeout: Duration,
    probe: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let permit = slot
        .try_acquire()
        .map_err(|_| "resource probe is still running; no extra probe started".to_owned())?;
    let task = tokio::task::spawn_blocking(move || {
        // Keep the slot in the worker, even when the caller times out. Blocking
        // OS calls cannot be forcibly cancelled and must not accumulate workers.
        let _permit = permit;
        probe()
    });
    tokio::time::timeout(timeout, task)
        .await
        .map_err(|_| {
            "resource probe timed out; results unavailable, OS call may still be finishing"
                .to_owned()
        })?
        .map_err(|error| format!("resource probe worker failed: {error}"))
}

pub async fn report(path: std::path::PathBuf) -> Result<String, String> {
    bounded_probe(&PROBE_SLOT, Duration::from_secs(3), move || sample(&path)).await
}

#[cfg(windows)]
fn sample(path: &std::path::Path) -> String {
    use cdr_windows_native::resources::{
        active_processor_count, cpu_times, disk_bytes, memory_bytes,
    };
    let clock = std::time::Instant::now();
    let (cpu, (memory, disk, processors)) = cpu::sample(
        cpu_times,
        || (memory_bytes(), disk_bytes(path), active_processor_count()),
        || clock.elapsed(),
        std::thread::sleep,
    );
    let mut lines = vec![match cpu.elapsed {
        Some(elapsed) => format!("Host resources · CPU sample {:.2}s", elapsed.as_secs_f64()),
        None => "Host resources · CPU sample unavailable".into(),
    }];
    lines.push(match cpu.percent {
        Ok(value) => format!("CPU: {value:.2}%"),
        Err(error) => format!("CPU: 조회 실패 · {error}"),
    });
    lines.push(match processors {
        Ok(count) if count > 64 => {
            format!("논리 CPU: {count} · CPU%는 primary processor group 범위이며 전체 CPU가 아님")
        }
        Ok(count) => format!("논리 CPU: {count}"),
        Err(error) => format!("CPU 측정 범위 미확인: {error}"),
    });
    lines.push(match memory {
        Ok(value) => format!(
            "RAM: 사용 {} / 전체 {} · 사용 가능 {}",
            gib(value.total - value.available),
            gib(value.total),
            gib(value.available)
        ),
        Err(error) => format!("RAM: 조회 실패 · {error}"),
    });
    lines.push(match disk {
        Ok(value) => format!(
            "Disk: 사용자 할당 전체 {} · 사용자 사용 가능 {} · 볼륨 free {}",
            gib(value.caller_total),
            gib(value.caller_available),
            gib(value.volume_free)
        ),
        Err(error) => format!("Disk: 조회 실패 · {error}"),
    });
    lines.join("\n")
}

#[cfg(not(windows))]
fn sample(_path: &std::path::Path) -> String {
    "Host resources: 조회 불가 · Windows 실측 API만 지원".into()
}

#[cfg(windows)]
fn gib(bytes: u64) -> String {
    let hundredths = u128::from(bytes) * 100 / (1_u128 << 30);
    format!("{}.{:02} GiB", hundredths / 100, hundredths % 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[tokio::test]
    async fn timed_out_worker_keeps_single_slot_until_actual_exit() {
        static SLOT: Semaphore = Semaphore::const_new(1);
        let (release, blocked) = mpsc::channel();
        let result = bounded_probe(&SLOT, Duration::from_millis(20), move || {
            blocked.recv_timeout(Duration::from_secs(2)).unwrap();
        })
        .await;
        assert!(result.unwrap_err().contains("timed out"));
        assert!(
            bounded_probe(&SLOT, Duration::from_secs(1), || panic!("extra worker"))
                .await
                .unwrap_err()
                .contains("still running")
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if SLOT.available_permits() == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            bounded_probe(&SLOT, Duration::from_secs(1), || 7)
                .await
                .unwrap(),
            7
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn actual_report_shows_units_and_independent_disk_failure() {
        let _guard = TEST_HOST_PROBE.lock().await;
        let root = tempfile::tempdir().unwrap();
        let report = report(root.path().join("absent")).await.unwrap();
        assert!(report.contains("CPU"));
        assert!(report.contains("RAM"));
        assert!(report.contains("GiB"));
        assert!(report.contains("GetDiskFreeSpaceExW"));
        assert!(report.contains("Disk: 조회 실패"));
    }
}
