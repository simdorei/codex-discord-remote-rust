//! Read-only diagnostics: never initialize/migrate missing or incompatible stores.
use rusqlite::{Connection, OpenFlags};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::Semaphore;
mod idle_release;
#[path = "protocol_report.rs"]
mod protocol;

static DIAGNOSTIC_READER: Semaphore = Semaphore::const_new(1);
static QUEUE_READER: Semaphore = Semaphore::const_new(1);

pub async fn queue_report(path: PathBuf) -> Result<String, String> {
    crate::context_view::bounded_reader(&QUEUE_READER, Duration::from_secs(3), move || {
        database("runner_queue", &path, &["codex_turn_queue"], true)
    })
    .await
}

pub struct Paths {
    pub state: PathBuf,
    pub mirror: PathBuf,
    pub bridge: PathBuf,
}

pub async fn report(paths: Paths) -> Result<String, String> {
    crate::context_view::bounded_reader(&DIAGNOSTIC_READER, Duration::from_secs(3), move || {
        let mut lines = vec![format!(
            "Rust diagnostic · {} {} · version {}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            env!("CARGO_PKG_VERSION")
        )];
        lines.push(database("state_db", &paths.state, &["threads"], false));
        lines.push(database(
            "mirror_db",
            &paths.mirror,
            &["mirror_threads", "codex_turn_queue"],
            true,
        ));
        lines.push(json_file("bridge_state", &paths.bridge));
        lines.push(idle_release::report(&paths.mirror));
        if let Some(home) = paths.state.parent() {
            lines.push(readable("session_index", &home.join("session_index.jsonl")));
            lines.push(json_file(
                "global_state",
                &home.join(".codex-global-state.json"),
            ));
        }
        lines.push("권한: 읽기 검사만 수행; 쓰기/삭제 권한과 DB 전체 무결성은 미검증".into());
        lines.push(protocol::report());
        lines.push("Codex 앱 창·앱 업데이트: 미검증 (서버 연결·프로토콜 등록과 별도)".into());
        lines.join("\n")
    })
    .await
}

fn database(label: &str, path: &Path, tables: &[&str], bridge_schema: bool) -> String {
    let result = (|| -> Result<String, String> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(|error| error.to_string())?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if bridge_schema && version != cdr_store::schema::LATEST_STORE_SCHEMA_VERSION {
            return Err(format!(
                "schema version {version}; supported {} (migration not attempted)",
                cdr_store::schema::LATEST_STORE_SCHEMA_VERSION
            ));
        }
        let mut counts = Vec::new();
        for table in tables {
            // Identifiers are fixed internal constants, never user-supplied SQL.
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(|error| error.to_string())?;
            counts.push(format!("{table}={count}"));
        }
        Ok(format!(
            "read OK · schema={version} · {}",
            counts.join(", ")
        ))
    })();
    match result {
        Ok(value) => format!("{label}: {value} · {}", path.display()),
        Err(error) => format!("{label}: 조회 실패 · {error} · {}", path.display()),
    }
}

fn readable(label: &str, path: &Path) -> String {
    let result = std::fs::File::open(path).and_then(|mut file| {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(std::io::Error::other("not a regular file"));
        }
        if metadata.len() > 0 {
            file.read_exact(&mut [0_u8; 1])?;
        }
        Ok(metadata.len())
    });
    match result {
        Ok(bytes) => format!("{label}: readable · {bytes} bytes (content not inspected)"),
        Err(error) => format!("{label}: 조회 실패 · {error}"),
    }
}

fn json_file(label: &str, path: &Path) -> String {
    let result = (|| -> Result<(), String> {
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        if !file
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("not a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > 1_048_576 {
            return Err("JSON exceeds diagnostic 1MiB read budget".into());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "invalid JSON at line {}, column {} (content withheld)",
                error.line(),
                error.column()
            )
        })?;
        if !value.is_object() {
            return Err("JSON root is not an object".into());
        }
        Ok(())
    })();
    match result {
        Ok(()) => format!("{label}: readable JSON object (values withheld)"),
        Err(error) => format!("{label}: 조회 실패 · {error}"),
    }
}
