use std::collections::HashMap;

use serde_json::Value;

const SUMMARY_DELTA_METHOD: &str = "item/reasoning/summaryTextDelta";
const ITEM_COMPLETED_METHOD: &str = "item/completed";
const MAX_ACTIVE_ITEMS: usize = 128;
const MAX_ITEM_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ItemKey {
    thread: String,
    turn: String,
    item: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommentaryBlock {
    pub thread_id: String,
    pub turn_id: String,
    pub text: String,
}

#[derive(Default)]
pub struct CommentaryBuffer {
    items: HashMap<ItemKey, String>,
}

impl CommentaryBuffer {
    pub fn observe(&mut self, method: &str, params: &Value) -> Option<CommentaryBlock> {
        match method {
            SUMMARY_DELTA_METHOD => {
                self.append_delta(params);
                None
            }
            ITEM_COMPLETED_METHOD => self.complete(params),
            _ => None,
        }
    }

    #[must_use]
    pub fn active_items(&self) -> usize {
        self.items.len()
    }

    pub fn discard_turn(&mut self, thread_id: &str, turn_id: &str) {
        self.items
            .retain(|key, _| key.thread != thread_id || key.turn != turn_id);
    }

    fn append_delta(&mut self, params: &Value) {
        let Some(key) = delta_key(params) else {
            return;
        };
        let Some(delta) = params.get("delta").and_then(Value::as_str) else {
            return;
        };
        if !self.items.contains_key(&key) && self.items.len() >= MAX_ACTIVE_ITEMS {
            return;
        }
        append_bounded(self.items.entry(key).or_default(), delta);
    }

    fn complete(&mut self, params: &Value) -> Option<CommentaryBlock> {
        let key = ItemKey {
            thread: text(params, "threadId")?.to_owned(),
            turn: text(params, "turnId")?.to_owned(),
            item: params
                .get("item")
                .and_then(|item| text(item, "id"))?
                .to_owned(),
        };
        self.items.remove(&key);
        let item = params.get("item")?;
        if cdr_app_server::async_questions::is_async_message(item) {
            return None;
        }
        let value = if text(item, "type") == Some("agentMessage") {
            if text(item, "phase") != Some("commentary") {
                return None;
            }
            text(item, "text")?.to_owned()
        } else {
            // Reasoning summaries are not user-facing progress replies.
            return None;
        };
        let text = value.trim();
        (!text.is_empty()).then(|| CommentaryBlock {
            thread_id: key.thread,
            turn_id: key.turn,
            text: text.to_owned(),
        })
    }
}

fn delta_key(params: &Value) -> Option<ItemKey> {
    Some(ItemKey {
        thread: text(params, "threadId")?.to_owned(),
        turn: text(params, "turnId")?.to_owned(),
        item: text(params, "itemId")?.to_owned(),
    })
}

fn text<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn append_bounded(target: &mut String, delta: &str) {
    let remaining = MAX_ITEM_BYTES.saturating_sub(target.len());
    if remaining == 0 {
        return;
    }
    if delta.len() <= remaining {
        target.push_str(delta);
        return;
    }
    let boundary = delta
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(delta.len()))
        .take_while(|index| *index <= remaining)
        .last()
        .unwrap_or(0);
    target.push_str(&delta[..boundary]);
}
