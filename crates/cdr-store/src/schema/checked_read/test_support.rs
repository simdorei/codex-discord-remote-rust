//! Thread-local deterministic boundaries for temporary store tests only.
use std::cell::RefCell;

use rusqlite::Connection;

use crate::{Result, completion_work::Source};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Boundary {
    Opened,
    Catalog,
    PageRead(Source),
    OrphanPreflight,
    OrphanMaterialized(usize, usize),
    BeforeCommit,
}

type Hook = Box<dyn FnMut(Boundary, &Connection) -> Result<()>>;
thread_local! {
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub(crate) fn boundary(point: Boundary, connection: &Connection) -> Result<()> {
    HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(hook) = slot.as_mut() {
            hook(point, connection)
        } else {
            Ok(())
        }
    })
}

pub(crate) struct HookGuard;

impl HookGuard {
    pub(crate) fn install(hook: impl FnMut(Boundary, &Connection) -> Result<()> + 'static) -> Self {
        HOOK.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(Box::new(hook));
        });
        Self
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|slot| {
            let _ = slot.borrow_mut().take();
        });
    }
}

pub(crate) fn catalog_cached(connection: &Connection) -> Result<bool> {
    let signature = super::super::catalog_cache::signature(connection)?;
    Ok(super::super::catalog_cache::contains(&signature))
}
