//! Lazy bounded metadata reads. Owned results become usable only after finish.
use std::path::Path;

use super::{Cursor, Page, Source, page_in};
use crate::{Result, StoreError, schema::checked_read::CheckedRead};

#[derive(Debug)]
pub struct PageRead {
    pub cursor: Cursor,
    pub page: Page,
}

pub struct MetadataReadRound<'a> {
    path: &'a Path,
    runtime: &'a str,
    generation: i64,
    snapshot: Option<CheckedRead>,
    failed: Option<StoreError>,
    sources: Vec<Source>,
    preflight_used: bool,
}

fn invalid_round() -> StoreError {
    StoreError::Integrity(
        "metadata round has no valid read snapshot; no results may be published".into(),
    )
}

impl MetadataReadRound<'_> {
    /// At most one page per source. The input cursor is never changed here.
    pub fn page(&mut self, source: Source, cursor: &Cursor) -> Result<PageRead> {
        if self.failed.is_some() {
            return Err(invalid_round());
        }
        if self.sources.len() >= Source::ALL.len() || self.sources.contains(&source) {
            return Err(StoreError::Integrity(
                "metadata round permits each of its eight sources once".into(),
            ));
        }
        self.sources.push(source);
        if self.snapshot.is_none() {
            match CheckedRead::open(self.path) {
                Ok(snapshot) => self.snapshot = Some(snapshot),
                Err(error) => {
                    self.failed = Some(error);
                    return Err(invalid_round());
                }
            }
        }
        let Some(snapshot) = self.snapshot.as_ref() else {
            return Err(invalid_round());
        };
        if let Err(error) = snapshot.ensure_active() {
            self.failed = Some(error);
            return Err(invalid_round());
        }
        let mut next = cursor.clone();
        let result = (|| {
            let page = page_in(
                snapshot.connection(),
                source,
                &mut next,
                self.runtime,
                self.generation,
            )?;
            #[cfg(test)]
            crate::schema::checked_read::test_support::boundary(
                crate::schema::checked_read::test_support::Boundary::PageRead(source),
                snapshot.connection(),
            )?;
            Ok(PageRead { cursor: next, page })
        })();
        if let Err(error) = snapshot.ensure_active() {
            self.failed = Some(error);
            return Err(invalid_round());
        }
        result
    }

    /// Optional negative-only sidecar for the current orphan page. The caller
    /// must stage it with the page and discard it if source/round completion fails.
    pub fn unprovable_orphans(&mut self, entries: &[super::Entry]) -> Result<Vec<usize>> {
        if self.failed.is_some() {
            return Err(invalid_round());
        }
        if self.preflight_used
            || !self.sources.contains(&Source::AsyncOrphan)
            || entries.len() > super::PAGE_SIZE
            || entries
                .iter()
                .any(|entry| entry.source != Source::AsyncOrphan)
        {
            return Err(StoreError::Integrity(
                "orphan preflight requires one bounded current source page".into(),
            ));
        }
        self.preflight_used = true;
        let Some(snapshot) = self.snapshot.as_ref() else {
            return Err(invalid_round());
        };
        if let Err(error) = snapshot.ensure_active() {
            self.failed = Some(error);
            return Err(invalid_round());
        }
        #[cfg(test)]
        let preflight = crate::schema::checked_read::test_support::boundary(
            crate::schema::checked_read::test_support::Boundary::OrphanPreflight,
            snapshot.connection(),
        );
        #[cfg(not(test))]
        let preflight = Ok(());
        let result = preflight.and_then(|()| {
            let targets = entries
                .iter()
                .map(|entry| entry.target.as_str())
                .collect::<Vec<_>>();
            crate::async_resolution::unprovable_in(snapshot.connection(), &targets)
        });
        if let Err(error) = snapshot.ensure_active() {
            self.failed = Some(error);
            return Err(invalid_round());
        }
        result
    }

    fn finish(self) -> Result<()> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        if let Some(snapshot) = self.snapshot {
            snapshot.finish()?;
        }
        Ok(())
    }
}

/// Run synchronous speculative discovery, then finish its single checked snapshot.
/// No requested page means no connection. The callback must stage its changes:
/// only an Ok return authorizes publication of hints, never execution or delivery.
pub fn read_round<T>(
    path: &Path,
    runtime: &str,
    generation: i64,
    operation: impl FnOnce(&mut MetadataReadRound<'_>) -> T,
) -> Result<T> {
    let mut round = MetadataReadRound {
        path,
        runtime,
        generation,
        snapshot: None,
        failed: None,
        sources: Vec::new(),
        preflight_used: false,
    };
    let value = operation(&mut round);
    round.finish()?;
    Ok(value)
}

#[cfg(test)]
mod preflight_tests;
#[cfg(test)]
mod tests;
