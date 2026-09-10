use std::fs;
use std::path::Path;

#[test]
fn offline_soak_sources_exclude_live_runtime_and_network_boundaries() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source = String::new();
    let soak_dir = manifest.join("src/soak");
    for entry in fs::read_dir(&soak_dir).expect("read soak source directory") {
        let path = entry.expect("read soak source entry").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            source.push_str(&fs::read_to_string(&path).expect("read soak Rust source"));
        }
    }
    source.push_str(
        &fs::read_to_string(manifest.join("src/bin/cdr-offline-soak.rs"))
            .expect("read offline soak binary source"),
    );

    for forbidden in [
        "discord_runtime::run",
        "GatewayRuntime",
        "DiscordSessionMirrorSender",
        "twilight_http",
        "reqwest",
    ] {
        assert!(
            !source.contains(forbidden),
            "offline soak source crossed live boundary: {forbidden}"
        );
    }
}
