use super::{http_gate, message_fixture::MessageFixture};
use crate::mirror_sync::{MirrorChannel, MirrorFuture, MirrorInventoryThread, MirrorTransport};
use serde_json::json;
use std::fmt::Write as _;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use twilight_model::channel::ChannelType;

pub(crate) struct Remote {
    pub creates: AtomicUsize,
}
fn channel(id: u64) -> MirrorChannel {
    MirrorChannel {
        id,
        guild_id: Some(1),
        parent_id: (id != 100).then_some(100),
        kind: if id == 100 {
            ChannelType::GuildText
        } else {
            ChannelType::PublicThread
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
        Box::pin(async { Ok(vec![channel(100)]) })
    }
    fn thread_inventory(&self, _: u64, _: u64) -> MirrorFuture<'_, Vec<MirrorInventoryThread>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn delete(&self, _: u64) -> MirrorFuture<'_, ()> {
        panic!("new must not delete existing rooms")
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
                (1, Some(100), ChannelType::PublicThread)
            );
            self.creates.fetch_add(1, Ordering::SeqCst);
            Ok(channel(43))
        })
    }
}

pub(crate) async fn setup(
    temp: &tempfile::TempDir,
) -> (MessageFixture, Arc<Remote>, http_gate::HttpGate) {
    setup_with_channels(temp, vec![42, 43]).await
}

pub(crate) async fn setup_with_channels(
    temp: &tempfile::TempDir,
    channels: Vec<u64>,
) -> (MessageFixture, Arc<Remote>, http_gate::HttpGate) {
    let gate = http_gate::start_for_channels(channels).await;
    let fixture = MessageFixture::new(
        temp,
        Arc::new(
            twilight_http::Client::builder()
                .token("fixture-token".into())
                .proxy(gate.address.clone(), true)
                .ratelimiter(None)
                .timeout(std::time::Duration::from_secs(5))
                .build(),
        ),
    )
    .await;
    let cwd = temp.path().to_string_lossy().into_owned();
    rusqlite::Connection::open(temp.path().join("state.sqlite"))
        .unwrap()
        .execute("UPDATE threads SET cwd=? WHERE id='thread-b'", [&cwd])
        .unwrap();
    cdr_store::mapping::upsert_thread(
        fixture.executor.mirror_db(),
        "thread-b",
        &cwd,
        "project",
        100,
        42,
        1.0,
    )
    .unwrap();
    cdr_store::mapping::upsert_project(
        fixture.executor.mirror_db(),
        &cwd,
        "project",
        100,
        1.0,
        |a, b| a == b,
    )
    .unwrap();
    let remote = Arc::new(Remote {
        creates: AtomicUsize::new(0),
    });
    fixture
        .executor
        .set_mirror_transport(remote.clone(), Some(1))
        .unwrap();
    (fixture, remote, gate)
}

pub(crate) fn persist(temp: &tempfile::TempDir, prompt: &str) {
    let cwd = temp.path().to_string_lossy().into_owned();
    let path = temp.path().join("new-rollout.jsonl");
    let lines = [
        json!({"type":"session_meta","payload":{"id":"new-thread","cwd":cwd}}),
        json!({"type":"turn_context","payload":{"turn_id":"first-turn"}}),
        json!({"type":"response_item","payload":{"role":"user","content":[{"text":prompt}]}}),
    ];
    std::fs::write(
        &path,
        lines.iter().fold(String::new(), |mut text, line| {
            writeln!(text, "{line}").unwrap();
            text
        }),
    )
    .unwrap();
    rusqlite::Connection::open(temp.path().join("state.sqlite")).unwrap()
        .execute("INSERT INTO threads VALUES ('new-thread','new',?,50,?,'model-a','high',0,0,0,'app-server','user')",
            rusqlite::params![cwd,path.to_string_lossy()]).unwrap();
}
