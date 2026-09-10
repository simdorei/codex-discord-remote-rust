use serde_json::{Map, Value};

mod collect;
mod delivery_origin;
mod helpers;
mod poll_result;
mod recent_text;
mod runner;
mod sender;
mod turn_context;

pub use collect::collect_items;
pub(crate) use collect::collect_items_with_context;
pub(crate) use delivery_origin::{discord_active_turn, discord_origin_user};
pub(crate) use poll_result::TargetFailures;
pub use poll_result::{SessionMirrorError, SessionMirrorPoll};
pub(crate) use recent_text::{RecentTextCache, normalized_text_digest};
pub use runner::{
    SessionMirrorFailureReport, SessionMirrorRetryDecision, SessionMirrorRetryState,
    run_session_mirror_worker,
};
pub(crate) use sender::assistant_scope;
pub use sender::{
    DiscordSessionMirrorSender, SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN,
    SESSION_MIRROR_EVENT_NONCE_DOMAIN, SessionMirrorDeliveryIdentity, SessionMirrorSender,
    session_delivery_identity,
};
pub(crate) use turn_context::read_context_before;

pub type SessionEvent = Map<String, Value>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MirrorDetail {
    Send,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MirrorKind {
    User,
    Commentary,
    Final,
    Aborted,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MirrorItem {
    pub digest: String,
    pub kind: MirrorKind,
    pub phase: String,
    pub text: String,
    pub turn_id: Option<String>,
    pub(crate) dedupe_recent_text: bool,
}

#[must_use]
pub fn format_item(item: &MirrorItem) -> String {
    match item.kind {
        MirrorKind::User => format!("Codex app user\n\n{}", item.text),
        MirrorKind::Commentary => format!("In progress\n\n{}", item.text),
        MirrorKind::Final => format!("Final\n\n{}", item.text),
        MirrorKind::Failed => format!("Failed\n\n{}", item.text),
        MirrorKind::Aborted => item.text.clone(),
    }
}
