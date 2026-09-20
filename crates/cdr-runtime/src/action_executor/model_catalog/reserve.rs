//! Explicit, account-observed Reserve selection; not automatic quota recovery.
//! A quota snapshot is not a grant: the app-server/backend still authorize each request.
use super::{ActionError, validate_effort};
use serde_json::Value;

pub(crate) const MODEL: &str = "gpt-reserve";

pub(crate) fn requested(value: &str) -> bool {
    [MODEL, "reserve", "luna-reserve", "Luna Reserve"]
        .iter()
        .any(|name| value.trim().eq_ignore_ascii_case(name))
}

fn invalid(reason: &str) -> ActionError {
    ActionError::Invalid(format!(
        "Luna Reserve: {reason}; no settings update was sent"
    ))
}

/// Read only a complete current account response, never a sparse notification/cache.
pub(crate) fn snapshot(rates: &Value) -> Result<&Value, ActionError> {
    let buckets = rates
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("quota information unavailable"))?;
    let mut candidates = buckets
        .values()
        .filter(|row| row.get("limitName").and_then(Value::as_str) == Some(MODEL));
    let row = candidates
        .next()
        .ok_or_else(|| invalid("no Reserve quota returned for this account"))?;
    if candidates.next().is_some() {
        return Err(invalid("ambiguous Reserve quota"));
    }
    if row
        .get("normalModelSlug")
        .and_then(Value::as_str)
        .is_none_or(|name| name.trim().is_empty() || name == MODEL)
    {
        return Err(invalid("normal-model metadata unavailable"));
    }
    Ok(row)
}

fn validate_capacity(row: &Value) -> Result<(), ActionError> {
    if row
        .get("rateLimitReachedType")
        .is_some_and(|v| !v.is_null())
        || row
            .get("spendControlReached")
            .is_some_and(|v| !v.is_null() && v != false)
    {
        return Err(invalid("Reserve is blocked by the server"));
    }
    let mut observed = false;
    for key in ["primary", "secondary"] {
        let Some(window) = row.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        let used = window
            .get("usedPercent")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| invalid("invalid quota window"))?;
        if used >= 100.0 {
            return Err(invalid("Reserve quota exhausted"));
        }
        observed = true;
    }
    if !observed {
        return Err(invalid("Reserve capacity is unknown"));
    }
    Ok(())
}

/// Borrow display/effort metadata while preserving the backend-provided quota alias.
pub(crate) fn catalog(catalog: &Value, rates: &Value) -> Result<Value, ActionError> {
    let quota = snapshot(rates)?;
    validate_capacity(quota)?;
    let normal = quota["normalModelSlug"]
        .as_str()
        .expect("validated metadata");
    let mut matches = super::rows(catalog).filter(|row| super::name(row) == Some(normal));
    let mut alias = matches
        .next()
        .cloned()
        .ok_or_else(|| invalid("normal model missing from model/list"))?;
    if matches.next().is_some() {
        return Err(invalid("normal model is ambiguous"));
    }
    alias["id"] = quota["limitName"].clone();
    alias["model"] = quota["limitName"].clone();
    alias["displayName"] = Value::String("Luna Reserve".into());
    let mut result = catalog.clone();
    let data = result
        .get_mut("data")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("model catalog unavailable"))?;
    data.retain(|row| super::name(row) != Some(MODEL));
    data.push(alias);
    Ok(result)
}

pub(crate) fn effort(
    catalog: &Value,
    explicit: Option<&str>,
    previous: Option<&str>,
) -> Result<String, ActionError> {
    if let Some(effort) = explicit {
        validate_effort(catalog, MODEL, effort)?;
        return Ok(effort.into());
    }
    if let Some(effort) = previous.filter(|effort| validate_effort(catalog, MODEL, effort).is_ok())
    {
        return Ok(effort.into());
    }
    let default = super::rows(catalog)
        .find(|row| super::name(row) == Some(MODEL))
        .and_then(|row| row.get("defaultReasoningEffort"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("supported default reasoning effort unavailable"))?;
    validate_effort(catalog, MODEL, default)?;
    Ok(default.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn rates() -> Value {
        json!({"rateLimitsByLimitId":{"base_model_inference":{"limitName":"gpt-reserve","normalModelSlug":"gpt-5.6-luna","primary":{"usedPercent":1},"secondary":null}}})
    }
    fn models() -> Value {
        json!({"data":[{"model":"gpt-5.6-luna","defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"medium"},{"reasoningEffort":"xhigh"}]}]})
    }
    #[test]
    fn reserve_alias_uses_normal_metadata_but_not_normal_routing() {
        let catalog = catalog(&models(), &rates()).unwrap();
        assert_eq!(
            super::super::canonical_model(&catalog, "Luna Reserve").unwrap(),
            MODEL
        );
        assert_eq!(
            super::super::canonical_model(&catalog, "gpt-5.6-luna").unwrap(),
            "gpt-5.6-luna"
        );
        assert_eq!(effort(&catalog, None, Some("max")).unwrap(), "medium");
        assert_eq!(effort(&catalog, None, Some("xhigh")).unwrap(), "xhigh");
        assert!(effort(&catalog, Some("max"), None).is_err());
    }
    #[test]
    fn reserve_missing_malformed_exhausted_and_duplicate_fail_closed() {
        assert!(catalog(&models(), &json!({})).is_err());
        for bad in [Value::Null, json!(-1), json!("1"), json!(100), json!(101)] {
            let mut rates = rates();
            rates["rateLimitsByLimitId"]["base_model_inference"]["primary"]["usedPercent"] = bad;
            assert!(catalog(&models(), &rates).is_err());
        }
        let mut duplicate = rates();
        duplicate["rateLimitsByLimitId"]["duplicate"] =
            duplicate["rateLimitsByLimitId"]["base_model_inference"].clone();
        assert!(catalog(&models(), &duplicate).is_err());
        let mut blocked = rates();
        blocked["rateLimitsByLimitId"]["base_model_inference"]["spendControlReached"] = json!(true);
        assert!(catalog(&models(), &blocked).is_err());
    }
    #[test]
    fn reserve_does_not_guess_missing_hidden_or_ambiguous_normal_metadata() {
        let mut missing = rates();
        missing["rateLimitsByLimitId"]["base_model_inference"]["normalModelSlug"] = Value::Null;
        assert!(catalog(&models(), &missing).is_err());
        let mut hidden = models();
        hidden["data"][0]["hidden"] = json!(true);
        assert!(catalog(&hidden, &rates()).is_err());
        let mut duplicate = models();
        let row = duplicate["data"][0].clone();
        duplicate["data"].as_array_mut().unwrap().push(row);
        assert!(catalog(&duplicate, &rates()).is_err());
    }
}
