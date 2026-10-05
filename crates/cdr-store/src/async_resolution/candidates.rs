//! Bounded diagnostics never substitute for accepted terminal authority.
use super::{MAX_EVIDENCE_BYTES, Obligation, held};
use crate::Result;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

pub(super) fn conflicted(db: &Connection, row: &Obligation) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM cdr_async_terminal_candidates WHERE question_id=? AND revision=? AND kind='conflict')",
        params![row.question_id,row.revision], |r| r.get(0))?)
}

pub(super) fn retain(db: &Connection, row: &Obligation, kind: &str, raw: &str) -> Result<()> {
    if raw.len() > MAX_EVIDENCE_BYTES {
        return Err(held(
            &row.thread_id,
            "terminal candidate exceeds evidence bound",
        ));
    }
    let digest = hex::encode(Sha256::digest(raw.as_bytes()));
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM cdr_async_terminal_candidates WHERE question_id=? AND revision=? AND kind=? AND evidence_sha256=?)",
        params![row.question_id,row.revision,kind,digest], |r| r.get(0))?;
    if exists {
        return Ok(());
    }
    // Bounds apply over the question's lifetime, not once per Goal revision.
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM cdr_async_terminal_candidates WHERE question_id=? AND kind=?",
        params![row.question_id, kind],
        |r| r.get(0),
    )?;
    let limit = match kind {
        "unverified" => 8,
        "conflict" => 1,
        "replaced" => 2,
        _ => return Err(held(&row.thread_id, "unsupported terminal candidate kind")),
    };
    if count >= limit {
        if kind == "replaced" {
            return Err(held(
                &row.thread_id,
                "existing invalid proof must be retained for reconciliation",
            ));
        }
        // Never discard a previously accepted proof. Further unverified noise
        // is not authority; an existing conflict already preserves its hold.
        return Ok(());
    }
    db.execute("INSERT INTO cdr_async_terminal_candidates(question_id,revision,kind,evidence_sha256,evidence_text) VALUES(?,?,?,?,?)",
        params![row.question_id,row.revision,kind,digest,raw])?;
    Ok(())
}
