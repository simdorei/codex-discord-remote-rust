// Compile the independent collector before action wiring; not an Action test.
#[path = "../src/resource_report.rs"]
mod resource_report;

#[cfg(not(windows))]
#[tokio::test]
async fn unsupported_host_reports_unavailable_without_creating_the_probe_path() {
    let _guard = resource_report::TEST_HOST_PROBE.lock().await;
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent");
    let report = resource_report::report(missing.clone()).await.unwrap();
    assert_eq!(
        report,
        "Host resources: 조회 불가 · Windows 실측 API만 지원"
    );
    assert!(!missing.exists());
}
