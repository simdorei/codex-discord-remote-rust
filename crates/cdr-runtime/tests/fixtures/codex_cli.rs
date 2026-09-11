//! Only the offline test executable uses this synthetic CLI; never the live bot.
use std::{fs, io::Write, path::Path};
pub fn run(root: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("cli-calls.jsonl"))?;
    writeln!(log, "{}", serde_json::to_string(&args)?)?;
    if std::env::var_os("CDR_CLI_PROFILE_CAPTURE").is_some() {
        let mut profiles = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("cli-home.jsonl"))?;
        writeln!(
            profiles,
            "{}",
            serde_json::to_string(&std::env::var("CODEX_HOME").ok())?
        )?;
    }
    let key = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["plugin", "marketplace", "add", _] => "marketplace_add",
        ["plugin", "add", "codex-discord-remote@codex-discord-remote"] => "plugin_add",
        ["plugin", "marketplace", "list", "--json"] => "marketplace_list",
        ["plugin", "list", "--json"] => "plugin_list",
        _ => return Err("unexpected fixture Codex arguments".into()),
    };
    if std::env::var("CDR_CLI_FAIL").ok().as_deref() == Some(key) {
        if std::env::var_os("CDR_CLI_LARGE").is_some() {
            std::io::stdout().write_all(&vec![b'A'; 262_144])?;
            println!("STDOUT_TAIL");
            std::io::stderr().write_all(&vec![b'B'; 262_144])?;
            eprintln!("STDERR_TAIL");
        }
        println!("codex stdout diagnostic");
        eprintln!("error: unrecognized subcommand (fixture)");
        return Err("required Codex fixture command failed".into());
    }
    if std::env::var("CDR_CLI_WARN").ok().as_deref() == Some(key) {
        eprintln!("warning: UTF-8 plugin inventory 한글");
    }
    match key {
        "marketplace_list" => print!("{}", fs::read_to_string(root.join("marketplaces.json"))?),
        "plugin_list" => print!("{}", fs::read_to_string(root.join("plugins.json"))?),
        _ => println!("fixture registration completed"),
    }
    Ok(())
}
