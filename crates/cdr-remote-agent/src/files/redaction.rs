use std::sync::OnceLock;

use regex::{Captures, Regex};

const MASK: &str = "[REDACTED]";

pub fn redact(text: &str) -> String {
    let mut value = replace(text, pem(), MASK);
    value = replace(&value, aws_access(), MASK);
    value = replace_captures(&value, aws_secret(), |capture| {
        format!("{}{MASK}", &capture[1])
    });
    value = replace(&value, gcp_key(), MASK);
    value = replace_captures(&value, gcp_json(), |capture| {
        format!("{}{MASK}{}", &capture[1], &capture[3])
    });
    value = replace_captures(&value, database_url(), |capture| {
        format!("{}{MASK}", &capture[1])
    });
    value = replace_captures(&value, connection_credentials(), |capture| {
        format!("{}{}:{MASK}@", &capture[1], &capture[2])
    });
    value = replace_captures(&value, url_userinfo(), |capture| {
        format!("{}{MASK}@", &capture[1])
    });
    value = replace_captures(&value, query_secret(), |capture| {
        format!("{}{MASK}", &capture[1])
    });
    value = replace_captures(&value, bearer(), |capture| format!("{}{MASK}", &capture[1]));
    value = replace_captures(&value, generic_secret(), |capture| {
        let quote = capture.get(2).map_or("", |value| value.as_str());
        format!("{}{quote}{MASK}{quote}", &capture[1])
    });
    replace_captures(&value, high_entropy(), |capture| {
        let token = &capture[0];
        let variety = usize::from(token.chars().any(|value| value.is_ascii_digit()))
            + usize::from(token.chars().any(|value| value.is_ascii_uppercase()))
            + usize::from(token.chars().any(|value| value.is_ascii_lowercase()));
        if variety < 2
            || (token.contains('/') && !token.chars().any(|value| value.is_ascii_digit()))
        {
            token.to_owned()
        } else {
            MASK.to_owned()
        }
    })
}

fn replace(value: &str, regex: &Regex, replacement: &str) -> String {
    regex.replace_all(value, replacement).into_owned()
}

fn replace_captures(
    value: &str,
    regex: &Regex,
    replacement: impl Fn(&Captures<'_>) -> String,
) -> String {
    regex.replace_all(value, replacement).into_owned()
}

macro_rules! pattern {
    ($name:ident, $value:literal) => {
        fn $name() -> &'static Regex {
            static VALUE: OnceLock<Regex> = OnceLock::new();
            VALUE.get_or_init(|| Regex::new($value).expect("valid redaction pattern"))
        }
    };
}

pattern!(
    pem,
    r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----"
);
pattern!(aws_access, r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b");
pattern!(
    aws_secret,
    r"\b((?:aws_secret_access_key|AWS_SECRET_ACCESS_KEY)\s*[:=]\s*)([A-Za-z0-9/+=]{40})\b"
);
pattern!(gcp_key, r"\bAIza[0-9A-Za-z_-]{35}\b");
pattern!(
    gcp_json,
    r#"("(?:private_key_id|client_secret|client_email)"\s*:\s*")([^"]*)(")"#
);
pattern!(
    database_url,
    r"(?i)\b((?:DATABASE_URL|DB_URL|POSTGRES_URL|MYSQL_URL|MONGO(?:DB)?_URI)\s*=\s*)(\S+)"
);
pattern!(
    connection_credentials,
    r"\b([A-Za-z][\w+.-]*://)([^:/\s]+):([^@/\s]+)@"
);
pattern!(url_userinfo, r"(?i)\b(https?://)[^/@\s]+@");
pattern!(
    query_secret,
    r"(?i)([?&](?:access_token|api_key|apikey|auth|key|password|secret|token)=)([^&#\s]+)"
);
pattern!(bearer, r"(?i)\b(Bearer\s+)([A-Za-z0-9\-_.~+/]{10,}=*)");
pattern!(
    generic_secret,
    r#"(?i)\b((?:api[_-]?key|apikey|access[_-]?token|auth[_-]?token|secret|password|passwd|pwd)\s*[:=]\s*)(['"]?)([A-Za-z0-9\-_.\/+={}]{8,})['"]?"#
);
pattern!(high_entropy, r"\b[A-Za-z0-9+/_-]{32,}={0,2}\b");
