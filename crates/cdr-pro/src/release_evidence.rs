//! Public-safe release evidence. Passing this gate never implies live/browser readiness.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Write, path::Path};

pub const REQUIRED: &[&str] = &[
    "repository_revision_stable",
    "plugin_manifest",
    "remote_mcp_capability_contract",
    "host_installer_inventory_contract",
    "browser_evidence_contract",
    "pro_runtime_contract",
    "installed_plugin_inventory",
    "fresh_resident_preflight",
];
pub const DEFERRED: &[&str] = &[
    "in_app_browser_live_evidence",
    "chatgpt_tool_exposure",
    "post_restart_runtime",
    "other_platform_installer_contract",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Skipped,
    Stale,
    Malformed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Check {
    pub id: String,
    pub category: String,
    pub status: Status,
}
impl Check {
    #[must_use]
    pub fn new(id: &str, category: &str, status: Status) -> Self {
        Self {
            id: id.into(),
            category: category.into(),
            status,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub schema_version: u32,
    pub certification_scope: &'static str,
    pub repository_revision: String,
    pub workspace_state: String,
    pub host_platform: String,
    pub plugin_version: String,
    pub required_check_ids: &'static [&'static str],
    pub checks: Vec<Check>,
    pub pre_restart_ready: bool,
    pub release_ready: bool,
    pub deferred_check_ids: &'static [&'static str],
}
impl Evidence {
    #[must_use]
    pub fn new(
        revision: String,
        workspace: String,
        host: String,
        version: String,
        checks: Vec<Check>,
    ) -> Self {
        let checks = normalize(checks);
        let ready = checks.iter().all(|c| c.status == Status::Passed);
        Self {
            schema_version: 1,
            certification_scope: "pre_restart",
            repository_revision: revision,
            workspace_state: workspace,
            host_platform: host,
            plugin_version: version,
            required_check_ids: REQUIRED,
            checks,
            pre_restart_ready: ready,
            release_ready: false,
            deferred_check_ids: DEFERRED,
        }
    }
    #[must_use]
    pub fn summary(&self) -> String {
        let mut lines = vec![
            if self.pre_restart_ready {
                "PRE-RESTART READY"
            } else {
                "PRE-RESTART BLOCKED"
            }
            .to_owned(),
        ];
        lines.extend(
            self.checks
                .iter()
                .map(|c| format!("{:9} {}", format!("{:?}", c.status).to_uppercase(), c.id)),
        );
        lines.push(format!("DEFERRED  {}", DEFERRED.join(", ")));
        lines.push("RELEASE READY: NO (live Browser, ChatGPT exposure, and restart remain)".into());
        format!("{}\n", lines.join("\n"))
    }
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        atomic_write(path, &format!("{json}\n"))?;
        atomic_write(&path.with_extension("txt"), &self.summary())
    }
}

#[must_use]
pub fn normalize(checks: Vec<Check>) -> Vec<Check> {
    let mut by_id: BTreeMap<String, Vec<Check>> = BTreeMap::new();
    for check in checks {
        by_id.entry(check.id.clone()).or_default().push(check);
    }
    let unexpected = by_id.keys().any(|id| !REQUIRED.contains(&id.as_str()));
    let mut normalized: Vec<_> = REQUIRED
        .iter()
        .map(|id| match by_id.remove(*id).as_deref() {
            Some([one]) => one.clone(),
            None => Check::new(id, "contract", Status::Skipped),
            Some(_) => Check::new(id, "contract", Status::Malformed),
        })
        .collect();
    if unexpected {
        normalized[0] = Check::new(REQUIRED[0], "contract", Status::Malformed);
    }
    normalized
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    staged
        .write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    staged.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}
