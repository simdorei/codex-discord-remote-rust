use tokio::time::Instant;
use twilight_model::{
    gateway::{event::Event, payload::incoming::Ready},
    id::Id,
    oauth::{ApplicationFlags, PartialApplication},
    user::CurrentUser,
};

use super::gateway_identity::{GatewayIdentityTracker, publish_gateway_event_identity};
use super::ingress::GatewayIngressConfig;
use super::runtime_publication::{publish_decoded_event, publish_receive_error};
use super::{GatewayIdentity, GatewayIdentityConflict, GatewayRuntime};

pub(super) fn identity(user_id: u64, application_id: u64) -> GatewayIdentity {
    GatewayIdentity {
        user_id: Id::new(user_id),
        application_id: Id::new(application_id),
    }
}

fn ready_event(expected: GatewayIdentity) -> Event {
    Event::Ready(Ready {
        application: PartialApplication {
            flags: ApplicationFlags::empty(),
            id: expected.application_id,
        },
        guilds: Vec::new(),
        resume_gateway_url: "wss://gateway.discord.gg".into(),
        session_id: "session".into(),
        shard: None,
        user: CurrentUser {
            accent_color: None,
            avatar: None,
            banner: None,
            bot: true,
            discriminator: 0,
            email: None,
            flags: None,
            global_name: None,
            id: expected.user_id,
            locale: None,
            mfa_enabled: false,
            name: "bot".into(),
            premium_type: None,
            public_flags: None,
            verified: None,
        },
        version: 10,
    })
}

pub(super) fn publish_ready(expected: GatewayIdentity, tracker: &GatewayIdentityTracker) {
    publish_gateway_event_identity(&ready_event(expected), tracker);
}

#[test]
fn gi_01_late_subscriber_reads_current_identity_without_waiting() {
    let expected = identity(11, 22);
    let tracker = GatewayIdentityTracker::new();
    publish_ready(expected, &tracker);
    assert_eq!(tracker.subscribe_identity().snapshot(), Some(expected));
}

#[test]
fn gi_02_identity_and_first_fatal_conflict_survive_without_subscribers() {
    let expected = identity(33, 44);
    let conflicting = identity(34, 45);
    let tracker = GatewayIdentityTracker::new();
    publish_ready(expected, &tracker);
    publish_ready(conflicting, &tracker);

    assert_eq!(tracker.subscribe_identity().snapshot(), Some(expected));
    assert_eq!(
        tracker.subscribe_conflict().snapshot(),
        Some(GatewayIdentityConflict {
            established: expected,
            observed: conflicting,
        })
    );
}

#[test]
fn gi_03_ready_identity_is_visible_inside_publication_observer() {
    let expected = identity(55, 66);
    let (ingress, _receivers) =
        super::ingress::GatewayIngress::new(GatewayIngressConfig::default()).unwrap();
    let identity = ingress.subscribe_identity();

    super::runtime_publication::publish_decoded_event_with(
        ready_event(expected),
        Instant::now(),
        &ingress,
        || assert_eq!(identity.snapshot(), Some(expected)),
        |_| {},
    );
}

#[test]
fn gi_04_repeated_same_ready_is_a_no_op_without_conflict() {
    let expected = identity(77, 88);
    let tracker = GatewayIdentityTracker::new();
    publish_ready(expected, &tracker);
    publish_ready(expected, &tracker);
    assert_eq!(tracker.subscribe_identity().snapshot(), Some(expected));
    assert_eq!(tracker.subscribe_conflict().snapshot(), None);
}

#[tokio::test]
async fn gi_05_non_ready_receive_error_and_shutdown_retain_identity() {
    let expected = identity(99, 111);
    let mut runtime = GatewayRuntime::new_offline_for_test(GatewayIngressConfig::default())
        .expect("default ingress config is valid");
    let identity = runtime.subscribe_identity();
    let conflict = runtime.subscribe_identity_conflict();
    let mut receivers = runtime.take_ingress_receivers().unwrap();

    for event in [
        ready_event(expected),
        Event::Resumed,
        Event::GatewayClose(None),
    ] {
        publish_decoded_event(
            event,
            Instant::now(),
            &runtime.ingress,
            &runtime.ingress_publish_outcomes,
        );
    }
    publish_receive_error(
        0,
        "offline error".into(),
        &runtime.ingress,
        &runtime.ingress_publish_outcomes,
    );
    assert_eq!(
        receivers.receive_errors.try_recv().unwrap().message,
        "offline error"
    );
    runtime.shutdown().await.expect("empty task set shuts down");

    assert_eq!(identity.snapshot(), Some(expected));
    assert_eq!(conflict.snapshot(), None);
}
