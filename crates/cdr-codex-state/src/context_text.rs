use serde_json::Value;
use std::collections::VecDeque;

#[derive(Clone, Copy)]
pub enum RecentTextMode {
    Visible,
    UserAndFinal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextTextItem {
    pub label: &'static str,
    pub text: String,
    pub observed_at: Option<String>,
    pub truncated: bool,
}

pub(crate) struct RecentText {
    limit: usize,
    mode: RecentTextMode,
    items: VecDeque<ContextTextItem>,
}

impl RecentText {
    pub(crate) fn new(limit: usize, mode: RecentTextMode) -> Self {
        Self {
            limit: limit.clamp(1, 50),
            mode,
            items: VecDeque::new(),
        }
    }

    pub(crate) fn push(&mut self, event: &Value) {
        let Some(item) = extract(event) else {
            return;
        };
        // Select eligible records before they consume the bounded window.
        if matches!(self.mode, RecentTextMode::UserAndFinal)
            && !matches!(item.label, "user" | "assistant final")
        {
            return;
        }
        // Equal text is not proof of equal occurrence. Keep chronological
        // records, including dual representations without established identity.
        if self.items.len() == self.limit {
            self.items.pop_front();
        }
        self.items.push_back(item);
    }

    pub(crate) fn finish(self) -> Vec<ContextTextItem> {
        self.items.into_iter().collect()
    }
}

fn extract(event: &Value) -> Option<ContextTextItem> {
    let payload = &event["payload"];
    let event_type = event["type"].as_str()?;
    let kind = payload["type"].as_str()?;
    let phase = payload["phase"].as_str().unwrap_or("commentary");
    if phase == "analysis" {
        return None;
    }
    let (role, fragments): (&str, Vec<&str>) = match (event_type, kind) {
        ("event_msg", "user_message") => ("user", vec![payload["message"].as_str()?]),
        ("event_msg", "agent_message") => ("assistant", vec![payload["message"].as_str()?]),
        ("response_item", "message") => (
            payload["role"].as_str()?,
            payload["content"]
                .as_array()?
                .iter()
                .filter(|v| matches!(v["type"].as_str(), Some("input_text" | "output_text")))
                .filter_map(|v| v["text"].as_str())
                .collect(),
        ),
        ("response_item", "function_call") => {
            let name = payload["name"].as_str()?;
            let notice = if name.ends_with("request_user_input") {
                "[choice_required]"
            } else {
                let arguments: Value = serde_json::from_str(payload["arguments"].as_str()?).ok()?;
                if arguments["sandbox_permissions"] != "require_escalated" {
                    return None;
                }
                "[approval_required]"
            };
            return Some(ContextTextItem {
                label: "assistant interactive",
                text: notice.into(),
                observed_at: timestamp(event),
                truncated: false,
            });
        }
        _ => return None,
    };
    let label = match role {
        "user" => "user",
        "assistant" if matches!(phase, "final_answer" | "final") => "assistant final",
        "assistant" => "assistant commentary",
        _ => return None,
    };
    let mut text = String::new();
    for fragment in fragments {
        if !text.is_empty() {
            text.push('\n');
        }
        let remaining = 1501_usize.saturating_sub(text.chars().count());
        text.extend(fragment.chars().take(remaining));
        if text.chars().count() > 1500 {
            break;
        }
    }
    let truncated = text.chars().count() > 1500;
    let text = text
        .chars()
        .take(1500)
        .collect::<String>()
        .trim()
        .to_owned();
    if text.is_empty() {
        return None;
    }
    Some(ContextTextItem {
        label,
        text,
        observed_at: timestamp(event),
        truncated,
    })
}

fn timestamp(event: &Value) -> Option<String> {
    event["timestamp"]
        .as_str()
        .map(|v| v.chars().take(64).collect())
}
