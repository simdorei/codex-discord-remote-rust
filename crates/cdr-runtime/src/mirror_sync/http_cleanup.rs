use super::{
    DiscordMirrorTransport, MirrorInventoryThread, MirrorSyncError, channel_id, convert, invalid,
    request_error,
};
use std::collections::BTreeMap;
use twilight_model::id::Id;

impl DiscordMirrorTransport {
    pub(super) async fn read_thread_inventory(
        &self,
        guild: u64,
        parent: u64,
    ) -> Result<Vec<MirrorInventoryThread>, MirrorSyncError> {
        let bot = self
            .0
            .current_user()
            .await
            .map_err(request_error)?
            .model()
            .await
            .map_err(|e| invalid(&e.to_string()))?
            .id;
        let guild = Id::new_checked(guild).ok_or_else(|| invalid("zero guild id"))?;
        let parent = channel_id(parent)?;
        let active = self
            .0
            .active_threads(guild)
            .await
            .map_err(request_error)?
            .model()
            .await
            .map_err(|e| invalid(&e.to_string()))?;
        let mut threads = active
            .threads
            .into_iter()
            .filter(|c| c.parent_id == Some(parent))
            .map(|c| (c.id, c))
            .collect::<BTreeMap<_, _>>();
        let mut before: Option<String> = None;
        loop {
            let mut request = self.0.public_archived_threads(parent).limit(100);
            if let Some(timestamp) = before.as_deref() {
                request = request.before(timestamp);
            }
            let page = request
                .await
                .map_err(request_error)?
                .model()
                .await
                .map_err(|e| invalid(&e.to_string()))?;
            let next = page
                .threads
                .last()
                .and_then(|c| c.thread_metadata.as_ref())
                .map(|m| m.archive_timestamp.iso_8601().to_string());
            for channel in page.threads {
                threads.insert(channel.id, channel);
            }
            match page.has_more {
                Some(false) => break,
                Some(true) => {}
                None => return Err(invalid("archived thread response omitted has_more")),
            }
            if next.is_none() || next == before {
                return Err(invalid("archived thread pagination did not advance"));
            }
            before = next;
        }
        Ok(threads
            .into_values()
            .map(|c| MirrorInventoryThread {
                owned_by_bot: c.owner_id == Some(bot),
                channel: convert(c),
            })
            .collect())
    }
}
