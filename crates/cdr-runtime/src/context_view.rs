//! Bounded context file inspection off the async command loop.
use cdr_codex_state::{
    ContextReadBudget, ContextReadTarget, RecentTextMode, ThreadInfo, read_context_batch_with_mode,
};
use std::time::Duration;
use tokio::sync::Semaphore;

static READER: Semaphore = Semaphore::const_new(1);

#[cfg(test)]
#[path = "context_connected_tests.rs"]
mod connected_tests;

pub async fn render(
    threads: Vec<ThreadInfo>,
    refresh: bool,
    limit: usize,
) -> Result<String, String> {
    bounded_reader(&READER, Duration::from_secs(3), move || {
        render_blocking(&threads, refresh, limit, RecentTextMode::Visible)
    })
    .await
}

pub(crate) async fn render_status(thread: ThreadInfo) -> Result<String, String> {
    bounded_reader(&READER, Duration::from_secs(3), move || {
        render_blocking(&[thread], true, 5, RecentTextMode::UserAndFinal)
    })
    .await
}

pub(crate) async fn bounded_reader(
    slot: &'static Semaphore,
    timeout: Duration,
    read: impl FnOnce() -> String + Send + 'static,
) -> Result<String, String> {
    let permit = slot
        .try_acquire()
        .map_err(|_| "context reader is still running; no extra reader started".to_owned())?;
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        read()
    });
    tokio::time::timeout(timeout, worker)
        .await
        .map_err(|_| {
            "context read timed out; snapshot unavailable, OS read may still be finishing"
                .to_owned()
        })?
        .map_err(|error| format!("context reader failed: {error}"))
}

#[cfg(test)]
#[path = "context_view_tests.rs"]
mod tests;

fn render_blocking(
    threads: &[ThreadInfo],
    refresh: bool,
    limit: usize,
    mode: RecentTextMode,
) -> String {
    let targets = threads
        .iter()
        .map(|thread| ContextReadTarget {
            thread: thread.id.clone(),
            path: thread.rollout_path.clone(),
        })
        .collect::<Vec<_>>();
    let batch = read_context_batch_with_mode(
        &targets,
        ContextReadBudget::default(),
        50,
        refresh.then_some(limit.clamp(1, 50)),
        mode,
    );
    let mut lines = vec!["Codex context · 마지막 기록 조회, 현재 실시간 측정 아님".to_owned()];
    if threads.is_empty() {
        lines.push("활성 대화 없음".into());
    }
    let mut text_budget = 12_000;
    for (thread, entry) in threads.iter().zip(batch.entries) {
        lines.push(format!(
            "\n{} | {} | 마지막 저장 model={} effort={}",
            thread.id,
            clipped(&thread.title, 240),
            clipped(&thread.model, 80),
            clipped(&thread.reasoning_effort, 40)
        ));
        match entry.usage {
            Ok(usage) => lines.push(crate::context_report::usage_lines(
                usage.as_ref(),
                thread.tokens_used,
                chrono::Utc::now(),
            )),
            Err(error) => {
                lines.push(format!("조회 실패: {error}"));
                lines.push(crate::context_report::usage_lines(
                    None,
                    thread.tokens_used,
                    chrono::Utc::now(),
                ));
                continue;
            }
        }
        if refresh {
            if entry.recent_items.is_empty() {
                lines.push("최근 표시 가능한 대화 없음".into());
            }
            for item in entry.recent_items {
                if text_budget == 0 {
                    lines.push("최근 대화 표시 한도 초과 · 이후 내용 생략".into());
                    break;
                }
                let count = item.text.chars().count();
                let shown = count.min(text_budget);
                text_budget -= shown;
                lines.push(format!(
                    "[{}] {}{}",
                    item.label,
                    item.text.chars().take(shown).collect::<String>(),
                    if item.truncated || shown < count {
                        " … [잘림]"
                    } else {
                        ""
                    }
                ));
            }
        }
    }
    if batch.skipped > 0 {
        lines.push(format!("조회 한도 50개 · 미조회 대화: {}", batch.skipped));
    }
    lines.join("\n")
}

fn clipped(text: &str, limit: usize) -> String {
    let mut shown = text
        .chars()
        .take(limit)
        .collect::<String>()
        .replace(['\r', '\n'], " ");
    if text.chars().count() > limit {
        shown.push('…');
    }
    shown
}
