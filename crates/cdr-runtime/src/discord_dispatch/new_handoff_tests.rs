use super::custody::{self, StageOutcome, StageRequest};

#[test]
fn production_slash_admission_envelope_can_handoff_new_prompt() {
    use twilight_model::application::{
        command::CommandType,
        interaction::{
            InteractionType,
            application_command::{CommandData, CommandDataOption, CommandOptionValue},
        },
    };
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let data = CommandData {
        guild_id: None,
        id: twilight_model::id::Id::new(1),
        name: "new".into(),
        kind: CommandType::ChatInput,
        options: vec![CommandDataOption {
            name: "prompt".into(),
            value: CommandOptionValue::String("  새 작업  ".into()),
        }],
        resolved: None,
        target_id: None,
    };
    let work =
        cdr_discord::interaction::route_command(&data, InteractionType::ApplicationCommand, false)
            .unwrap()
            .work
            .unwrap();
    let StageOutcome::Created(mut staged) = custody::stage(
        &db,
        &StageRequest {
            application_id: 1,
            interaction_id: 30,
            channel_id: 99,
            user_id: 20,
            source_message_id: None,
            work: &work,
            settings_resolver: None,
        },
    )
    .unwrap() else {
        panic!("first slash must be admitted")
    };
    staged.acknowledge().unwrap();
    let receipt = staged.into_receipt();
    cdr_store::ingress::begin_execution(&db, &receipt.ingress_id, "processing", None, 1.0).unwrap();
    let cdr_discord::interaction::RoutedWork::Slash(invocation) = work else {
        panic!("slash")
    };
    let crate::command_plan::CommandAction::New { prompt } =
        crate::command_plan::plan_slash(&invocation).unwrap()
    else {
        panic!("new")
    };
    crate::message_worker::new_handoff_tests::handoff(&db, &receipt.ingress_id, &prompt);
}
