use rusqlite::{Connection, OpenFlags};
use std::{fmt::Write, path::Path};

pub(super) fn report(path: &Path) -> String {
    match read(path) {
        Ok(text) => text,
        Err(error) => format!("봇 연결 해제 상태: 조회 실패 · {error}"),
    }
}

fn read(path: &Path) -> rusqlite::Result<String> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(std::time::Duration::from_millis(250))?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cdr_idle_release')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok("봇 연결 해제 상태: 이전 저장 형식 (변경하지 않음)".into());
    }
    let mut stmt = db.prepare("SELECT state,COUNT(*) FROM cdr_idle_release WHERE state!='Settled' GROUP BY state ORDER BY state")?;
    let counts = stmt
        .query_map([], |r| {
            Ok(format!(
                "{}={}",
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut text = format!(
        "봇 연결 해제 상태: {}",
        if counts.is_empty() {
            "보류 없음".into()
        } else {
            counts.join(", ")
        }
    );
    let mut stmt = db.prepare("SELECT thread_id,state,detail FROM cdr_idle_release WHERE state IN ('Dispatching','Resubscribing','Unknown') ORDER BY thread_id LIMIT 3")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (thread, state, detail) = row?;
        let detail: String = detail.chars().take(160).collect();
        write!(text, "\n{thread} · {state}: {detail} · 자동 재실행 보류")
            .expect("writing to a String cannot fail");
    }
    Ok(text)
}
