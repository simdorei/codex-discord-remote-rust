use std::future::ready;

use cdr_discord::gateway::GatewayIdentity;
use cdr_discord::gateway::ingress::{GatewayIngress, GatewayIngressConfig, PublishOutcome};
use tokio::sync::watch;
use tokio::time::Instant;
use twilight_model::gateway::{event::Event, payload::incoming::Ready};
use twilight_model::id::Id;
use twilight_model::oauth::{ApplicationFlags, PartialApplication};
use twilight_model::user::CurrentUser;

use super::wait_for_identity;
use crate::discord_runtime::DiscordRuntimeError;

fn ready_event(user_id: u64, application_id: u64) -> Event {
    Event::Ready(Ready {
        application: PartialApplication {
            flags: ApplicationFlags::empty(),
            id: Id::new(application_id),
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
            id: Id::new(user_id),
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

#[tokio::test]
async fn tii_01_message_identity_wait_does_not_finish_before_ready() {
    let (ingress, _receivers) =
        GatewayIngress::new(GatewayIngressConfig::default()).expect("valid ingress");
    let mut identity = ingress.subscribe_identity();
    let mut conflict = ingress.subscribe_identity_conflict();
    let (_shutdown_tx, mut shutdown) = watch::channel(false);
    let mut waiting = Box::pin(wait_for_identity(
        &mut identity,
        &mut conflict,
        &mut shutdown,
    ));

    tokio::select! {
        biased;
        result = &mut waiting => panic!("identity wait finished before Ready: {result:?}"),
        () = ready(()) => {}
    }
    assert_eq!(
        ingress.publish(ready_event(11, 22), Instant::now()),
        PublishOutcome::Ignored
    );
    assert_eq!(
        waiting.await.expect("identity wait succeeds"),
        Some(GatewayIdentity {
            user_id: Id::new(11),
            application_id: Id::new(22),
        })
    );
}

#[tokio::test]
async fn tii_02_shutdown_ends_identity_wait_without_ready() {
    let (ingress, _receivers) =
        GatewayIngress::new(GatewayIngressConfig::default()).expect("valid ingress");
    let mut identity = ingress.subscribe_identity();
    let mut conflict = ingress.subscribe_identity_conflict();
    let (shutdown_tx, mut shutdown) = watch::channel(false);
    shutdown_tx.send(true).expect("shutdown receiver remains");

    assert_eq!(
        wait_for_identity(&mut identity, &mut conflict, &mut shutdown)
            .await
            .expect("shutdown is clean"),
        None
    );
}

#[tokio::test]
async fn tii_03_conflicting_ready_fails_before_returning_stale_identity() {
    let (ingress, _receivers) =
        GatewayIngress::new(GatewayIngressConfig::default()).expect("valid ingress");
    let mut identity = ingress.subscribe_identity();
    let mut conflict = ingress.subscribe_identity_conflict();
    let (_shutdown_tx, mut shutdown) = watch::channel(false);
    let _ = ingress.publish(ready_event(11, 22), Instant::now());
    let _ = ingress.publish(ready_event(33, 44), Instant::now());

    let error = wait_for_identity(&mut identity, &mut conflict, &mut shutdown)
        .await
        .expect_err("identity conflict is fatal");
    assert!(matches!(
        error,
        DiscordRuntimeError::GatewayIdentityConflict(_)
    ));
}
