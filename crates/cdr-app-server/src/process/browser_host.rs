use std::collections::{BTreeMap, HashMap};

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
) {
    // An explicit host setting selects a new source for the entire set. Do not
    // combine one host's pipe with another host's key or executor identifier.
    let source = if overrides.keys().any(|name| is_host_variable(name)) {
        overrides
    } else {
        inherited
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
        return;
    }
    if !ambiguous
        && values
            .iter()
            .all(|value| value.is_some_and(|value| !value.trim().is_empty()))
    {
        for (key, value) in HOST_VARIABLES.into_iter().zip(values) {
            environment.insert(key.to_owned(), value.expect("complete host set").to_owned());
        }
        return;
    }

    // A missing executor variable can select an unchecked legacy browser path.
    // Retain the access-check trigger even for an incomplete/ambiguous set.
    // No pipe/key is supplied, so the existing service reports host unavailable.
    let executor = if ambiguous {
        ""
    } else {
        values[1].unwrap_or("")
    };
    environment.insert(HOST_VARIABLES[1].to_owned(), executor.to_owned());
}
