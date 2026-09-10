use cdr_discord::text::{
    DISCORD_MAX_LEN, fit_single_message, split_delivery_chunks, split_message,
};

#[test]
fn empty_and_short_messages_match_python() {
    assert_eq!(split_message("  ", DISCORD_MAX_LEN), ["(no output)"]);
    assert_eq!(split_message("  hello\n", DISCORD_MAX_LEN), ["hello"]);
    assert_eq!(fit_single_message("  hello\n", 100), "hello");
}

#[test]
fn newline_is_preferred_and_unicode_is_never_split_inside_a_character() {
    assert_eq!(split_message("abcd\nefgh", 6), ["abcd", "efgh"]);
    assert_eq!(
        split_message("가나다라마바사", 3),
        ["가나다", "라마바", "사"]
    );
}

#[test]
fn marked_delivery_chunks_match_python_budget_and_stay_within_limit() {
    let chunks = split_delivery_chunks(&"x".repeat(DISCORD_MAX_LEN + 200), true);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].starts_with("[1/2]\n"));
    assert!(chunks[1].starts_with("[2/2]\n"));
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= DISCORD_MAX_LEN)
    );
    assert_eq!(split_delivery_chunks("hello", true), ["hello"]);
    assert_eq!(split_delivery_chunks("hello", false), ["hello"]);
}

#[test]
fn single_message_truncation_matches_existing_suffix_contract() {
    let fitted = fit_single_message(&"x".repeat(100), 40);
    assert_eq!(fitted.chars().count(), 40);
    assert!(fitted.ends_with("\n\n[truncated for Discord]"));
}
