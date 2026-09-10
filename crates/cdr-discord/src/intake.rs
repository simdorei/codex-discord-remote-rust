#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageFacts {
    pub message_content: MessageContentAccess,
    pub channel: Permission,
    pub user: Permission,
    pub bot_bridge_mention: bool,
    pub runtime: RuntimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageContentAccess {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Running,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateReason {
    Accepted,
    MessageContentDisabled,
    ChannelNotAllowed,
    UserNotAllowed,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateResult {
    pub handled: bool,
    pub bot_bridge_mention: bool,
    pub restart_notice: bool,
    pub reason: GateReason,
}

#[must_use]
pub fn gate_message(facts: MessageFacts) -> GateResult {
    if facts.message_content == MessageContentAccess::Disabled {
        return handled(GateReason::MessageContentDisabled, false, false);
    }
    if facts.channel == Permission::Denied {
        return handled(GateReason::ChannelNotAllowed, false, false);
    }
    if facts.user == Permission::Denied {
        return handled(GateReason::UserNotAllowed, facts.bot_bridge_mention, false);
    }
    if facts.runtime == RuntimeState::Stopping {
        return handled(GateReason::Stopping, facts.bot_bridge_mention, true);
    }
    GateResult {
        handled: false,
        bot_bridge_mention: facts.bot_bridge_mention,
        restart_notice: false,
        reason: GateReason::Accepted,
    }
}

fn handled(reason: GateReason, bot_bridge_mention: bool, restart_notice: bool) -> GateResult {
    GateResult {
        handled: true,
        bot_bridge_mention,
        restart_notice,
        reason,
    }
}
