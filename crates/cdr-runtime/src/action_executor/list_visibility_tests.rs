use super::{ThreadInfo, render_blocking};
use std::{collections::BTreeMap, path::PathBuf};

fn threads(count: usize) -> Vec<ThreadInfo> {
    (0..count)
        .map(|i| ThreadInfo {
            id: format!("visible-{i:02}"),
            title: format!("title {i}"),
            cwd: "C:/repo".into(),
            updated_at: 10,
            rollout_path: PathBuf::from("unavailable-fixture-rollout.jsonl"),
            model: "saved-model".into(),
            reasoning_effort: "high".into(),
            tokens_used: None,
            archived_at: 20,
        })
        .collect()
}

#[test]
fn failed_and_unloaded_observations_do_not_hide_local_records() {
    let states = BTreeMap::from([
        (
            "visible-00".into(),
            "조회 실패: other app owns writer".into(),
        ),
        (
            "visible-01".into(),
            "notLoaded (다른 앱 실행 여부 미확인)".into(),
        ),
    ]);
    let text = render_blocking(threads(3), None, 0, false, &states);
    for i in 0..3 {
        assert!(text.contains(&format!("visible-{i:02}")), "{text}");
    }
    assert!(text.contains("조회 실패: other app owns writer"));
    assert!(text.contains("notLoaded"));
    assert!(text.contains("현재 실행 상태 조회 안 됨"));
    assert!(text.contains("목록: 3/3개 표시"));
    assert!(text.contains("실행 권한 확인이 아닙니다"));
}

#[test]
fn display_limits_and_probe_budget_do_not_renumber_or_hide_the_inventory() {
    let all = render_blocking(threads(60), Some("visible-01"), 0, false, &BTreeMap::new());
    for i in 0..60 {
        assert!(all.contains(&format!("visible-{i:02}")));
    }
    assert!(all.contains("목록: 60/60개 표시"));
    assert!(all.contains("*2 |"));
    let limited = render_blocking(threads(60), Some("visible-01"), 2, false, &BTreeMap::new());
    assert!(limited.contains("visible-00"));
    assert!(limited.contains("visible-01"));
    assert!(!limited.contains("visible-02"));
    assert!(limited.contains("*2 |"));
    assert!(limited.contains("목록: 2/60개 표시"));
    assert!(limited.contains("다른 PC/CODEX_HOME은 포함하지 않음"));
}

#[test]
fn archived_list_identifies_its_scope_without_a_runtime_probe() {
    let text = render_blocking(threads(2), None, 0, true, &BTreeMap::new());
    assert!(text.contains("archived_at:"));
    assert!(text.contains("로컬 Codex DB의 아카이브된 대화"));
    assert!(!text.contains("state 미확인"));
}
