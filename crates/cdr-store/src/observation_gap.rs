//! Source-sequence coverage, not permission to replay a request or release a hold.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

mod ledger;
mod proof;
mod schema;
pub(crate) use ledger::scope_verified_in;
pub use ledger::{activate, discover, mark_unknown, next, scope_verified};
pub use proof::{Effect, certify, finish_page};
pub(crate) use schema::{migrate_schema, schema_current};
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;

pub const PAGE_SIZE: usize = 32;
const MAX_SPANS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub owner_id: String,
    pub generation: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub first: i64,
    pub last: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub id: i64,
    pub scope: Scope,
    pub first: i64,
    pub last: i64,
    pub cursor: i64,
    pub revision: i64,
    pub verified: Vec<Span>,
}
impl Gap {
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let json: String = r.get(8)?;
        let verified = serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(Self {
            id: r.get(0)?,
            scope: Scope {
                owner_id: r.get(1)?,
                generation: r.get(2)?,
            },
            first: r.get(3)?,
            last: r.get(4)?,
            cursor: r.get(5)?,
            revision: r.get(6)?,
            verified,
        })
    }
    #[must_use]
    pub fn contains_verified(&self, sequence: i64) -> bool {
        self.verified
            .iter()
            .any(|s| s.first <= sequence && sequence <= s.last)
    }
    fn add(&mut self, sequence: i64) -> Result<()> {
        if sequence < self.first || sequence > self.last {
            return Err(invalid("proof outside range"));
        }
        self.verified.push(Span {
            first: sequence,
            last: sequence,
        });
        self.verified.sort_by_key(|s| s.first);
        let mut merged: Vec<Span> = Vec::new();
        for span in self.verified.drain(..) {
            if let Some(last) = merged.last_mut()
                && span.first <= last.last.saturating_add(1)
            {
                last.last = last.last.max(span.last);
            } else {
                merged.push(span);
            }
        }
        if merged.len() > MAX_SPANS {
            return Err(invalid("observation proof span budget exhausted"));
        }
        self.verified = merged;
        Ok(())
    }
    fn complete(&self) -> bool {
        self.verified.as_slice()
            == [Span {
                first: self.first,
                last: self.last,
            }]
    }
}
const COLUMNS: &str =
    "gap_id,owner_id,generation,first_seq,last_seq,scan_cursor,revision,state,verified_json";
fn read_gap(db: &Connection, id: i64) -> Result<Option<Gap>> {
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS} FROM cdr_observation_gaps WHERE gap_id=?"),
            [id],
            Gap::read,
        )
        .optional()?)
}
fn active(db: &Connection, scope: &Scope) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM cdr_observation_streams WHERE owner_id=?1 AND generation=?2 AND active=1)",
        params![scope.owner_id,scope.generation], |r|r.get(0))?)
}
fn save(db: &Connection, expected: &Gap, next: &Gap) -> Result<bool> {
    Ok(db.execute("UPDATE cdr_observation_gaps SET scan_cursor=?1,revision=revision+1,state=?2,verified_json=?3
        WHERE gap_id=?4 AND owner_id=?5 AND generation=?6 AND first_seq=?7 AND last_seq=?8
        AND scan_cursor=?9 AND revision=?10",
        params![next.cursor,if next.complete() {"Verified"} else {"Open"},serde_json::to_string(&next.verified)?,
        expected.id,expected.scope.owner_id,expected.scope.generation,expected.first,expected.last,expected.cursor,expected.revision])? == 1)
}
fn invalid(message: &str) -> StoreError {
    StoreError::Integrity(message.into())
}
