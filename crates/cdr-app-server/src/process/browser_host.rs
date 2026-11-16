use std::collections::{BTreeMap, HashMap};

use crate::browser_host_diagnostics::{BrowserHostFields, BrowserHostLaunchObservation};

// This is connection metadata supplied by a host, not an authorization grant.
// The browser service must still verify the host signature and the current
// executor/session/turn. Never manufacture an identity or an allow response.
const HOST_VARIABLES: [&str; 3] = [
    "BROWSER_USE_ACCESS_PIPE_PATH",
    "BROWSER_USE_ACCESS_EXECUTOR_ID",
    "BROWSER_USE_ACCESS_VERIFYING_IDENTITY",
];

pub(super) fn is_host_variable(name: &str) -> bool {
    HOST_VARIABLES
        .iter()
        .any(|key| name.eq_ignore_ascii_case(key))
}

pub(super) fn apply(
    inherited: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
    environment: &mut HashMap<String, String>,
) -> BrowserHostLaunchObservation {
    // An explicit host setting selects a new source for the entire set. Do not
    // combine one host's pipe with another host's key or executor identifier.
    let source = if overrides.keys().any(|name| is_host_variable(name)) {
        overrides
    } else {
        inherited
    };
    let mut observation = BrowserHostLaunchObservation {
        source: if std::ptr::eq(source, overrides) {
            "explicit_override"
        } else {
            "inherited"
        },
        inherited: BrowserHostFields::from_environment(
            inherited.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        ),
        overrides: BrowserHostFields::from_environment(
            overrides.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        ),
        child_environment: BrowserHostFields::default(),
        forwarding_result: "absent",
    };
    let mut values = [None; 3];
    let mut ambiguous = false;
    for (name, value) in source {
        if let Some(index) = HOST_VARIABLES
            .iter()
            .position(|key| name.eq_ignore_ascii_case(key))
        {
            ambiguous |= values[index].is_some();
            values[index] = Some(value.as_str());
        }
    }
    if values.iter().all(Option::is_none) {
        observation.source = "none";
    } else if !ambiguous
        && values
            .iter()
            .all(|value| value.is_some_and(|value| !value.trim().is_empty()))
    {
        for (key, value) in HOST_VARIABLES.into_iter().zip(values) {
            environment.insert(key.to_owned(), value.expect("complete host set").to_owned());
        }
        observation.forwarding_result = "forwarded";
    } else {
        // Keep the existing guard trigger for incomplete/ambiguous contexts.
        let executor = if ambiguous {
            ""
        } else {
            values[1].unwrap_or("")
        };
        environment.insert(HOST_VARIABLES[1].to_owned(), executor.to_owned());
        observation.forwarding_result = if ambiguous {
            "held_ambiguous"
        } else {
            "held_partial"
        };
    }
    observation.child_environment = BrowserHostFields::from_environment(
        environment.iter().map(|(k, v)| (k.as_str(), v.as_str())),
    );
    observation
}
