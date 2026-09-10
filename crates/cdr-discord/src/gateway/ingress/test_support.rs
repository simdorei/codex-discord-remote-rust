use twilight_model::id::{Id, marker::ChannelMarker};

use super::{GatewayIngress, MessageGapStateError};

impl GatewayIngress {
    pub(in crate::gateway) fn force_message_gap_limits_for_test(
        &self,
        channel_id: Id<ChannelMarker>,
        observation_count: u64,
        revision: u64,
    ) -> Result<(), MessageGapStateError> {
        self.message_gaps
            .force_limits(channel_id, observation_count, revision)
    }

    pub(in crate::gateway) fn poison_message_gaps_for_test(&self) {
        self.message_gaps.poison();
    }
}
