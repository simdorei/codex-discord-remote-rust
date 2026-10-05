//! Keep the first RPC's stop snapshot across nested resubscription and pipe waits.
use serde_json::Value;
use std::collections::BTreeSet;
tokio::task_local! {
    static ORIGINAL: Option<Value>;
}

pub(super) fn is_set() -> bool {
    ORIGINAL.try_with(|_| ()).is_ok()
}

pub(in crate::manager) fn current() -> Option<Value> {
    ORIGINAL.try_with(Clone::clone).ok().flatten()
}

/// Called only after the Archive adapter has validated its server-side subtree.
/// Keep the first revision and root; do not capture a newer child origin.
pub(super) fn archive(
    root: &str,
    children: &BTreeSet<String>,
) -> Result<Option<Value>, crate::AppServerError> {
    if !is_set() {
        return Ok(None);
    }
    let mut frozen = current().unwrap_or_else(|| {
        serde_json::json!({
            "target":root,"stopRevision":0,
        })
    });
    if frozen.as_object().is_none_or(|value| value.len() != 2)
        || frozen["target"].as_str() != Some(root)
        || frozen["stopRevision"]
            .as_i64()
            .is_none_or(|revision| revision < 0)
        || root.is_empty()
        || root.trim() != root
        || children.len() > 100
        || children.contains(root)
        || children
            .iter()
            .any(|child| child.is_empty() || child.trim() != child)
    {
        return Err(crate::AppServerError::MutationHeld {
            message: "archive subtree differs from the original admission; no refresh or retarget"
                .into(),
        });
    }
    let mut targets = children.clone();
    targets.insert(root.to_owned());
    frozen["archiveTargets"] = serde_json::json!(targets);
    Ok(Some(frozen))
}

pub(super) async fn scope<T>(
    origin: Option<Value>,
    future: impl std::future::Future<Output = T>,
) -> T {
    ORIGINAL.scope(origin, future).await
}
