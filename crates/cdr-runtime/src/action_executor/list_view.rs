use cdr_app_server::ResidentAppServer;
use cdr_codex_state::{ContextReadBudget, ContextReadTarget, ThreadInfo, read_context_batch};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::Semaphore;

static LIST_READER: Semaphore = Semaphore::const_new(1);
#[cfg(test)]
#[path = "display_connected_tests.rs"]
mod connected_tests;
#[path = "thread_state_probe.rs"]
mod thread_state_probe;

pub(super) async fn states(
    server: Option<&ResidentAppServer>,
    threads: &[ThreadInfo],
    limit: u32,
) -> BTreeMap<String, String> {
    let Some(server) = server else {
        return BTreeMap::new();
    };
    thread_state_probe::read(server, threads, count(limit)).await
}

pub(super) async fn render(
    threads: Vec<ThreadInfo>,
    selected: Option<String>,
    limit: u32,
    archived: bool,
    states: BTreeMap<String, String>,
) -> Result<String, String> {
    crate::context_view::bounded_reader(&LIST_READER, Duration::from_secs(3), move || {
        render_blocking(threads, selected.as_deref(), limit, archived, &states)
    })
    .await
}

fn render_blocking(
    mut threads: Vec<ThreadInfo>,
    selected: Option<&str>,
    limit: u32,
    archived: bool,
    states: &BTreeMap<String, String>,
) -> String {
    for thread in &mut threads {
        thread.title = clipped(&thread.title, 100);
    }
    // Preserve numbering and workspace references computed over the complete list.
    let headers = super::format::format_threads(&threads, selected, limit);
    let targets = threads
        .iter()
        .take(count(limit))
        .map(|thread| ContextReadTarget {
            thread: thread.id.clone(),
            path: thread.rollout_path.clone(),
        })
        .collect::<Vec<_>>();
    let batch = (!archived).then(|| read_context_batch(&targets, ContextReadBudget::default(), 50));
    let mut lines = Vec::new();
    for (index, (thread, header)) in threads.iter().zip(headers.lines()).enumerate() {
        if archived {
            lines.push(format!(
                "{header} | archived_at: {}",
                timestamp(thread.archived_at)
            ));
            continue;
        }
        let entry = batch.as_ref().and_then(|batch| batch.entries.get(index));
        let usage = entry
            .and_then(|entry| entry.usage.as_ref().ok())
            .and_then(Option::as_ref);
        let ctx = usage.map_or_else(
            || "미확인/미확인".into(),
            |value| {
                format!(
                    "{}/{}",
                    tokens(value.last_input_tokens),
                    tokens(value.peak_input_tokens)
                )
            },
        );
        let cumulative = thread.tokens_used.and_then(|v| u64::try_from(v).ok());
        let used = cumulative.map_or_else(|| "미확인".into(), tokens);
        let recommend = cumulative.is_some_and(|v| v >= 50_000_000)
            || usage.is_some_and(|value| {
                value.last_input_tokens >= 200_000 || value.peak_input_tokens >= 200_000
            });
        let row = format!(
            "{header} | state {} | ctx {ctx} (마지막/최대 입력) | used {used} (누적) | rec {} | 마지막 저장 model {} effort {} | updated_at: {}",
            states
                .get(&thread.id)
                .map_or("미확인 (현재 실행 상태 조회 안 됨)", String::as_str),
            if recommend {
                "archive"
            } else if usage.is_none() || cumulative.is_none() {
                "미확인"
            } else {
                "-"
            },
            clipped(&thread.model, 80),
            clipped(&thread.reasoning_effort, 40),
            timestamp(thread.updated_at)
        );
        let evidence = match entry.map(|entry| &entry.usage) {
            Some(Err(error)) => format!("ctx 조회 실패: {error}"),
            Some(Ok(Some(value))) => format!(
                "ctx 관측 시각: {} · 실시간 아님",
                value
                    .observed_at
                    .as_deref()
                    .map_or_else(|| "미확인".into(), |value| clipped(value, 64))
            ),
            Some(Ok(None)) => "ctx 측정 기록 없음".into(),
            None => "ctx 미조회 (파일 조회 한도 50개)".into(),
        };
        lines.push(format!("{row} | {}", evidence.replace(['\r', '\n'], " ")));
    }
    lines.join("\n")
}

fn count(limit: u32) -> usize {
    if limit == 0 {
        usize::MAX
    } else {
        limit as usize
    }
}

pub(super) fn timestamp(value: i64) -> String {
    if value <= 0 {
        return "미확인".into();
    }
    chrono::DateTime::from_timestamp(value, 0)
        .map_or_else(|| "미확인".into(), |time| time.to_rfc3339())
}

fn tokens(value: u64) -> String {
    let (divisor, unit) = if value >= 1_000_000 {
        (1_000_000, "M")
    } else {
        (1_000, "K")
    };
    format!(
        "{}.{:03}{unit}",
        value / divisor,
        (value % divisor) * 1_000 / divisor
    )
}

fn clipped(value: &str, limit: usize) -> String {
    let result = value
        .chars()
        .take(limit)
        .collect::<String>()
        .replace(['\r', '\n'], " ");
    if value.chars().count() > limit {
        format!("{result}…")
    } else {
        result
    }
}
