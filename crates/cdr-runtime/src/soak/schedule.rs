use sha2::{Digest, Sha256};

const TARGETS: [&str; 2] = ["thread-a", "thread-b"];
pub(super) const SCHEDULE_VERSION: u32 = 1;

pub(super) struct DeterministicSchedule {
    state: u64,
    digest: Sha256,
}

impl DeterministicSchedule {
    pub fn new(seed: u64) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"cdr-offline-soak-schedule-v1\0");
        digest.update(seed.to_le_bytes());
        digest.update(b"recovery:a-unavailable:b-progress:a-recover\0");
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
            digest,
        }
    }

    pub fn target_order(&mut self) -> [&'static str; 2] {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        let order = if self.state & 1 == 0 {
            TARGETS
        } else {
            [TARGETS[1], TARGETS[0]]
        };
        self.digest.update(self.state.to_le_bytes());
        self.digest.update(order[0].as_bytes());
        self.digest.update([0]);
        self.digest.update(order[1].as_bytes());
        self.digest.update([0]);
        order
    }

    pub fn digest(&self) -> String {
        hex::encode(self.digest.clone().finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::DeterministicSchedule;

    #[test]
    fn same_seed_and_steps_have_same_digest_while_a_different_seed_does_not() {
        let digest = |seed| {
            let mut schedule = DeterministicSchedule::new(seed);
            let _ = schedule.target_order();
            let _ = schedule.target_order();
            schedule.digest()
        };
        assert_eq!(digest(42), digest(42));
        assert_ne!(digest(42), digest(43));
    }
}
