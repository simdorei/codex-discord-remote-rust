use super::*;

impl Remote {
    pub(super) async fn execute_new(
        &self,
        executor: &ActionExecutor<AppServerTurnBackend>,
        action: &CommandAction,
        context: ActionContext,
    ) -> Result<cdr_runtime::action_executor::ActionResult, cdr_runtime::action_executor::ActionError>
    {
        if let Some((entered, resume)) = &self.pause_create {
            let duplicate = async {
                entered.notified().await;
                let result = executor.execute_with_context(action.clone(), context).await;
                resume.notify_one();
                result
            };
            let (first, _duplicate) = Box::pin(tokio::time::timeout(
                std::time::Duration::from_secs(10),
                async {
                    tokio::join!(
                        executor.execute_with_context(action.clone(), context),
                        duplicate
                    )
                },
            ))
            .await
            .expect("new/duplicate fixture exceeded deadline");
            first
        } else {
            executor.execute_with_context(action.clone(), context).await
        }
    }
}

#[derive(Default)]
pub(super) struct Remote {
    pub(super) creates: Mutex<usize>,
    pub(super) fail: bool,
    pub(super) pause_create: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
fn channel(id: u64) -> MirrorChannel {
    MirrorChannel {
        id,
        guild_id: Some(1),
        parent_id: (id == 100 || id == 101).then_some(99),
        kind: if id == 100 || id == 101 {
            ChannelType::PublicThread
        } else {
            ChannelType::GuildText
        },
        name: "project".into(),
        topic: None,
        archived: false,
    }
}
impl MirrorTransport for Remote {
    fn channel(&self, id: u64) -> MirrorFuture<'_, Option<MirrorChannel>> {
        Box::pin(async move { Ok(Some(channel(id))) })
    }
    fn channels(&self, _: u64) -> MirrorFuture<'_, Vec<MirrorChannel>> {
        Box::pin(async { Ok(vec![channel(99)]) })
    }
    fn thread_inventory(&self, _: u64, _: u64) -> MirrorFuture<'_, Vec<MirrorInventoryThread>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn delete(&self, _: u64) -> MirrorFuture<'_, ()> {
        panic!("new must never clean up other rooms")
    }
    fn update<'a>(
        &'a self,
        _: &'a MirrorChannel,
        _: &'a str,
        _: Option<&'a str>,
        _: bool,
    ) -> MirrorFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn create<'a>(
        &'a self,
        guild: u64,
        parent: Option<u64>,
        kind: ChannelType,
        _: &'a str,
        _: Option<&'a str>,
    ) -> MirrorFuture<'a, MirrorChannel> {
        Box::pin(async move {
            assert_eq!(
                (guild, parent, kind),
                (1, Some(99), ChannelType::PublicThread)
            );
            *self.creates.lock().unwrap() += 1;
            if let Some((entered, resume)) = &self.pause_create {
                entered.notify_one();
                resume.notified().await;
            }
            if self.fail {
                return Err(MirrorSyncError::Discord("HTTP 403".into()));
            }
            Ok(channel(100))
        })
    }
}
