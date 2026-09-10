//! Read-only comparison of live Discord mirror channels with persisted mappings.
use cdr_runtime::mirror_sync::{DiscordMirrorTransport, MirrorTransport};
use cdr_runtime::{
    config::RuntimeConfig,
    runtime_paths::{RuntimePaths, discover_inputs},
    startup::{StartupArgs, load_environment},
};
use std::collections::BTreeSet;
use std::sync::Arc;
use twilight_model::{channel::ChannelType, id::Id};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: mirror_inventory <env-path> <origin-channel-id>".into());
    }
    let startup = StartupArgs::parse(["--env".into(), args[0].clone().into()])?;
    let env = load_environment(&startup, &std::env::current_exe()?)?;
    let config = RuntimeConfig::from_map(&env, startup.cli)?;
    let paths = RuntimePaths::resolve(&env, &discover_inputs(&env, std::env::current_dir()?)?)?;
    let db = rusqlite::Connection::open_with_flags(
        &paths.mirror_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut statement = db.prepare("SELECT discord_channel_id FROM mirror_projects")?;
    let projects = statement
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(u64::try_from)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut statement = db.prepare("SELECT discord_thread_id FROM mirror_threads")?;
    let mapped = statement
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(u64::try_from)
        .collect::<Result<BTreeSet<_>, _>>()?;
    let http = Arc::new(twilight_http::Client::new(
        config.bot_token.expose().to_owned(),
    ));
    let origin = Id::new_checked(args[1].parse()?).ok_or("zero channel ID")?;
    let channel = http.channel(origin).await?.model().await?;
    let guild = channel.guild_id.ok_or("origin has no guild")?;
    let channels = http.guild_channels(guild).await?.models().await?;
    let categories = channels
        .iter()
        .filter(|c| c.kind == ChannelType::GuildCategory && c.name.as_deref() == Some("Codex"))
        .map(|c| c.id)
        .collect::<BTreeSet<_>>();
    let mirror_projects = channels
        .iter()
        .filter(|c| {
            c.kind == ChannelType::GuildText
                && (c.parent_id.is_some_and(|p| categories.contains(&p))
                    || c.topic
                        .as_deref()
                        .is_some_and(|t| t.starts_with("Codex project mirror:")))
        })
        .map(|c| c.id.get())
        .collect::<BTreeSet<_>>();
    let active = http.active_threads(guild).await?.model().await?;
    let orphan_threads = active
        .threads
        .iter()
        .filter(|c| {
            c.parent_id
                .is_some_and(|p| mirror_projects.contains(&p.get()))
                && !mapped.contains(&c.id.get())
        })
        .map(|c| c.id.get())
        .collect::<Vec<_>>();
    println!(
        "stored_projects={} live_mirror_projects={} unmapped_project_ids={:?}",
        projects.len(),
        mirror_projects.len(),
        mirror_projects.difference(&projects).collect::<Vec<_>>()
    );
    println!(
        "stored_threads={} unmapped_active_thread_ids={orphan_threads:?}",
        mapped.len()
    );
    let remote = DiscordMirrorTransport::new(http);
    let mut all_orphans = Vec::new();
    for parent in &mirror_projects {
        for thread in remote.thread_inventory(guild.get(), *parent).await? {
            if thread.owned_by_bot && !mapped.contains(&thread.channel.id) {
                all_orphans.push(thread.channel.id);
            }
        }
    }
    println!("owned_unmapped_active_and_archived_ids={all_orphans:?} no_mutations=true");
    Ok(())
}
