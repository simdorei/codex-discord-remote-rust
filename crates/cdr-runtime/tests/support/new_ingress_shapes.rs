use cdr_runtime::{command_plan::CommandAction, message_plan::MessagePlan};
use cdr_store::ingress::{self, IngressKind, NewIngress};
use std::path::Path;

pub fn admit(
    db: &Path,
    kind: IngressKind,
    action: &CommandAction,
    channel: u64,
) -> (String, serde_json::Value) {
    let (key, payload) = match kind {
        IngressKind::Message => (
            "message:30",
            serde_json::json!({
                "version":1,"content":"!new 새 작업","plan":MessagePlan::Execute(action.clone()),
                "attachments":[],"processing_mode":"normal","author_is_bot":false,
                "routing":{"mirrored_target":null,"selected_target":"resolved_before_processing"},
            }),
        ),
        IngressKind::Interaction => {
            use cdr_discord::interaction::route_command;
            use twilight_model::application::{
                command::CommandType,
                interaction::{
                    InteractionType,
                    application_command::{CommandData, CommandDataOption, CommandOptionValue},
                },
            };
            let command = CommandData {
                guild_id: None,
                id: twilight_model::id::Id::new(1),
                name: "new".into(),
                kind: CommandType::ChatInput,
                options: vec![CommandDataOption {
                    name: "prompt".into(),
                    value: CommandOptionValue::String("새 작업".into()),
                }],
                resolved: None,
                target_id: None,
            };
            let work = route_command(&command, InteractionType::ApplicationCommand, false)
                .unwrap()
                .work
                .unwrap();
            (
                "interaction:30",
                serde_json::json!({"version":1,"processing_mode":"normal","work":work}),
            )
        }
        IngressKind::Action => panic!("use actual gateway kinds"),
    };
    let admitted = ingress::admit(
        db,
        &NewIngress {
            ingress_id: key.into(),
            kind,
            event_id: Some(30),
            application_id: (kind == IngressKind::Interaction).then_some(1),
            channel_id: i64::try_from(channel).unwrap(),
            owner_user_id: 20,
            source_message_id: (kind == IngressKind::Message).then_some(30),
            payload: payload.clone(),
            target_thread_id: None,
            canonical_owner: None,
            now: 1.0,
        },
    )
    .unwrap()
    .record
    .unwrap()
    .payload;
    // Admission adds routing authority. All raw transport fields must remain
    // byte-for-byte equivalent; execution must preserve this admitted envelope.
    let mut transport = admitted.clone();
    let origin = transport
        .as_object_mut()
        .unwrap()
        .remove("new_origin")
        .unwrap();
    assert_eq!(origin["channel"], channel);
    assert_eq!(transport, payload);
    ingress::begin_execution(db, key, "processing", None, 2.0).unwrap();
    (key.into(), admitted)
}
