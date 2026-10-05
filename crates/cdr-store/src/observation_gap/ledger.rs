use super::{
    COLUMNS, Connection, Gap, OptionalExtension, Path, Result, Scope, TransactionBehavior, active,
    invalid, open_initialized, params,
};

/// Installed before observer intake. No new UUID retires an unsealed old stream.
pub fn activate(path: &Path, scope: &Scope) -> Result<()> {
    if scope.owner_id.is_empty() || scope.generation < 0 {
        return Err(invalid("invalid observation scope"));
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<bool> = tx
        .query_row(
            "SELECT active FROM cdr_observation_streams WHERE owner_id=?1 AND generation=?2",
            params![scope.owner_id, scope.generation],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(is_active) = existing {
        if !is_active {
            return Err(invalid("retired observation scope cannot reactivate"));
        }
        tx.commit()?;
        return Ok(());
    }
    let (count, active_count, maximum): (i64, i64, i64) = tx.query_row(
        "SELECT COUNT(*),COALESCE(SUM(active),0),COALESCE(MAX(generation),-1)
         FROM cdr_observation_streams WHERE owner_id=?1",
        [&scope.owner_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if count > 0 && (active_count == 0 || scope.generation <= maximum) {
        return Err(invalid("retired owner or stale observation generation"));
    }
    tx.execute("INSERT OR IGNORE INTO cdr_observation_gaps
        (owner_id,generation,first_seq,last_seq,scan_cursor,state,detail)
        SELECT owner_id,generation,0,0,0,'Unresolved','previous unsealed stream; missing tail is not reconstructed'
        FROM cdr_observation_streams WHERE unsealed=1 AND (owner_id!=?1 OR generation!=?2)",
        params![scope.owner_id,scope.generation])?;
    let missing_tail: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_observation_streams s
         WHERE s.unsealed=1 AND (s.owner_id!=?1 OR s.generation!=?2)
         AND NOT EXISTS(SELECT 1 FROM cdr_observation_gaps g
             WHERE g.owner_id=s.owner_id AND g.generation=s.generation
             AND g.first_seq=0 AND g.last_seq=0 AND g.state='Unresolved'))",
        params![scope.owner_id, scope.generation],
        |r| r.get(0),
    )?;
    if missing_tail {
        return Err(invalid(
            "previous unsealed observation tail was not preserved",
        ));
    }
    tx.execute(
        "UPDATE cdr_observation_streams SET active=0 WHERE owner_id!=?1 OR generation!=?2",
        params![scope.owner_id, scope.generation],
    )?;
    tx.execute(
        "INSERT INTO cdr_observation_streams(owner_id,generation,active) VALUES(?1,?2,1)
        ON CONFLICT(owner_id,generation) DO UPDATE SET active=1",
        params![scope.owner_id, scope.generation],
    )?;
    tx.commit()?;
    Ok(())
}

/// Capture only newly seen sequence space. A claimed G1 upper is never enlarged.
pub fn discover(path: &Path, scope: &Scope, upper: i64) -> Result<()> {
    if !(0..i64::MAX).contains(&upper) {
        return Err(invalid("source sequence exhausted"));
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !active(&tx, scope)? {
        return Err(invalid("observation scope is not active"));
    }
    let seen: i64 = tx.query_row(
        "SELECT seen_seq FROM cdr_observation_streams WHERE owner_id=?1 AND generation=?2",
        params![scope.owner_id, scope.generation],
        |r| r.get(0),
    )?;
    // Two original-window consumers may commit in reverse capture order. An
    // older exact scope snapshot is included, not a backwards source stream.
    if upper <= seen {
        tx.commit()?;
        return Ok(());
    }
    let inserted = tx.execute(
        "INSERT INTO cdr_observation_gaps
        (owner_id,generation,first_seq,last_seq,scan_cursor,state)
        VALUES(?1,?2,?3,?4,?5,'Open')",
        params![scope.owner_id, scope.generation, seen + 1, upper, seen],
    )?;
    if inserted != 1 {
        return Err(invalid("observation range was not inserted"));
    }
    let id = tx.last_insert_rowid();
    let advanced = tx.execute(
        "UPDATE cdr_observation_streams SET seen_seq=?3
        WHERE owner_id=?1 AND generation=?2 AND active=1 AND seen_seq=?4",
        params![scope.owner_id, scope.generation, upper, seen],
    )?;
    if advanced != 1 {
        return Err(invalid("observation checkpoint CAS lost"));
    }
    let saved: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_observation_gaps g
        JOIN cdr_observation_streams s ON s.owner_id=g.owner_id AND s.generation=g.generation
        WHERE g.gap_id=?1 AND g.owner_id=?2 AND g.generation=?3
        AND g.first_seq=?4 AND g.last_seq=?5 AND g.scan_cursor=?6
        AND g.revision=0 AND g.state='Open' AND g.verified_json='[]'
        AND s.active=1 AND s.seen_seq=?5)",
        params![id, scope.owner_id, scope.generation, seen + 1, upper, seen],
        |r| r.get(0),
    )?;
    if !saved {
        return Err(invalid(
            "observation range/checkpoint identity was not preserved",
        ));
    }
    tx.commit()?;
    Ok(())
}
pub fn mark_unknown(path: &Path, scope: &Scope, detail: &str) -> Result<()> {
    let detail: String = detail.chars().take(512).collect();
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT OR IGNORE INTO cdr_observation_gaps
        (owner_id,generation,first_seq,last_seq,scan_cursor,state,detail)
        VALUES(?1,?2,0,0,0,'Unresolved',?3)",
        params![scope.owner_id, scope.generation, detail],
    )?;
    // Zero affected rows is valid only for an exact, still-unresolved duplicate.
    let saved:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_observation_gaps
         WHERE owner_id=?1 AND generation=?2 AND first_seq=0 AND last_seq=0 AND state='Unresolved')",
        params![scope.owner_id,scope.generation],|r|r.get(0))?;
    if !saved {
        return Err(invalid("unattributed observation gap was not preserved"));
    }
    tx.commit()?;
    Ok(())
}

/// Persisted finite-cycle keyset: later G2 cannot keep postponing the next G1 pass.
pub fn next(path: &Path, scope: &Scope) -> Result<Option<Gap>> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !active(&tx, scope)? {
        return Ok(None);
    }
    let (mut after,mut upper):(i64,i64)=tx.query_row(
        "SELECT scan_after,cycle_upper FROM cdr_observation_streams WHERE owner_id=?1 AND generation=?2",
        params![scope.owner_id,scope.generation],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let mut found = None;
    for _ in 0..2 {
        found = tx
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM cdr_observation_gaps
            WHERE owner_id=?1 AND generation=?2 AND state='Open' AND first_seq>0
            AND gap_id>?3 AND gap_id<=?4 ORDER BY gap_id LIMIT 1"
                ),
                params![scope.owner_id, scope.generation, after, upper],
                Gap::read,
            )
            .optional()?;
        if found.is_some() {
            break;
        }
        after = 0;
        upper=tx.query_row("SELECT COALESCE(MAX(gap_id),0) FROM cdr_observation_gaps WHERE owner_id=?1 AND generation=?2",
            params![scope.owner_id,scope.generation],|r|r.get(0))?;
    }
    tx.execute("UPDATE cdr_observation_streams SET scan_after=?3,cycle_upper=?4 WHERE owner_id=?1 AND generation=?2",
        params![scope.owner_id,scope.generation,after,upper])?;
    if let Some(gap) = &mut found
        && gap.cursor == gap.last
    {
        tx.execute("UPDATE cdr_observation_gaps SET scan_cursor=first_seq-1,revision=revision+1 WHERE gap_id=?",
            [gap.id])?;
        gap.cursor = gap.first - 1;
        gap.revision += 1;
    }
    tx.commit()?;
    Ok(found)
}

/// No target-local exception: any unidentified old/current hole keeps optional idle held.
pub fn scope_verified(path: &Path, scope: &Scope, through: i64) -> Result<bool> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let verified = scope_verified_in(&tx, scope, through)?;
    tx.commit()?;
    Ok(verified)
}
pub(crate) fn scope_verified_in(db: &Connection, scope: &Scope, through: i64) -> Result<bool> {
    let seen:Option<i64>=db.query_row(
        "SELECT seen_seq FROM cdr_observation_streams WHERE owner_id=?1 AND generation=?2 AND active=1",
        params![scope.owner_id,scope.generation],|r|r.get(0)).optional()?;
    let Some(seen) = seen else {
        return Ok(false);
    };
    if through < 0 || through > seen || !(0..i64::MAX).contains(&seen) {
        return Ok(false);
    }
    let old_unsealed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_observation_streams
         WHERE unsealed=1 AND (owner_id!=?1 OR generation!=?2))",
        params![scope.owner_id, scope.generation],
        |r| r.get(0),
    )?;
    if old_unsealed {
        return Ok(false);
    }
    let unresolved: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_observation_gaps WHERE state!='Verified')",
        [],
        |r| r.get(0),
    )?;
    if unresolved {
        return Ok(false);
    }
    // Absence is not positive coverage. Check every contiguous original range,
    // including its actual complete spans, under the caller's single snapshot.
    let mut statement = db.prepare(&format!(
        "SELECT {COLUMNS} FROM cdr_observation_gaps WHERE owner_id=?1 AND generation=?2
        AND first_seq>0 AND state='Verified' ORDER BY first_seq,gap_id"
    ))?;
    let mut rows = statement.query(params![scope.owner_id, scope.generation])?;
    let mut expected = 1;
    while let Some(row) = rows.next()? {
        let gap = Gap::read(row)?;
        if gap.first != expected || gap.last > seen || !gap.complete() {
            return Ok(false);
        }
        expected = gap.last + 1;
    }
    Ok(expected == seen + 1)
}
