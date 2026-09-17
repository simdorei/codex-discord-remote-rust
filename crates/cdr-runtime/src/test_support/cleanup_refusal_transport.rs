//! Action/worker boundary fixture; real store/guard behavior is tested separately.
use crate::mirror_sync::{
    MirrorChannel, MirrorFuture, MirrorInventoryThread, MirrorSyncError, MirrorTransport,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use twilight_model::channel::ChannelType;

#[derive(Default)]
pub(crate) struct RefusingTransport {
    pub calls: AtomicUsize,
}

impl MirrorTransport for RefusingTransport {
    fn channels(&self, _: u64) -> MirrorFuture<'_, Vec<MirrorChannel>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(MirrorSyncError::CleanupProtected {
                channel: 31,
                reason: "ingress",
            })
        })
    }
    fn channel(&self, _: u64) -> MirrorFuture<'_, Option<MirrorChannel>> {
        panic!("unexpected lookup")
    }
    fn thread_inventory(&self, _: u64, _: u64) -> MirrorFuture<'_, Vec<MirrorInventoryThread>> {
        panic!("unexpected inventory")
    }
    fn delete(&self, _: u64) -> MirrorFuture<'_, ()> {
        panic!("refusal must not delete")
    }
    fn create<'a>(
        &'a self,
        _: u64,
        _: Option<u64>,
        _: ChannelType,
        _: &'a str,
        _: Option<&'a str>,
    ) -> MirrorFuture<'a, MirrorChannel> {
        panic!("unexpected create")
    }
    fn update<'a>(
        &'a self,
        _: &'a MirrorChannel,
        _: &'a str,
        _: Option<&'a str>,
        _: bool,
    ) -> MirrorFuture<'a, ()> {
        panic!("unexpected update")
    }
}
