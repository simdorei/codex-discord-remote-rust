PRAGMA user_version = 2;

CREATE TABLE busy_choices (
    choice_id TEXT PRIMARY KEY,
    owner_user_id INTEGER NOT NULL,
    channel_id INTEGER NOT NULL,
    target_thread_id TEXT,
    prompt TEXT NOT NULL,
    allow_steer INTEGER NOT NULL,
    created_at REAL NOT NULL,
    expires_at REAL NOT NULL,
    claimed_at REAL
);

CREATE TABLE chatgpt_app_mirror_conversations (
    conversation_id TEXT PRIMARY KEY,
    primed_at REAL NOT NULL
);

CREATE TABLE chatgpt_app_mirror_events (
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
    seen_at REAL NOT NULL,
    PRIMARY KEY(conversation_id, message_id)
);

CREATE TABLE chatgpt_app_mirror_slots (
    slot_index INTEGER PRIMARY KEY CHECK(slot_index BETWEEN 1 AND 5),
    conversation_id TEXT UNIQUE NOT NULL,
    discord_thread_id INTEGER UNIQUE NOT NULL,
    created_at REAL NOT NULL
);

CREATE TABLE codex_session_mirror_events (
    event_digest TEXT PRIMARY KEY,
    codex_thread_id TEXT NOT NULL,
    created_at REAL NOT NULL
);

CREATE TABLE codex_session_mirror_offsets (
    codex_thread_id TEXT PRIMARY KEY,
    rollout_path TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    updated_at REAL NOT NULL
);

CREATE TABLE codex_thread_transport_affinity (
    codex_thread_id TEXT PRIMARY KEY,
    transport TEXT NOT NULL CHECK(transport IN ('desktop')),
    updated_at REAL NOT NULL
);

CREATE TABLE codex_turn_attempts (
    attempt_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    app_server_generation INTEGER NOT NULL,
    app_server_process_id INTEGER,
    target_thread_id TEXT NOT NULL,
    client_request_id TEXT UNIQUE,
    state TEXT NOT NULL CHECK(state IN ('exec_pending', 'start_prewrite', 'start_unknown', 'running', 'turn_terminal', 'needs_review')),
    baseline_turn_ids TEXT NOT NULL,
    turn_id TEXT,
    last_error TEXT NOT NULL DEFAULT '',
    progress_at REAL NOT NULL,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    UNIQUE(job_id, attempt_number)
);

CREATE TABLE codex_turn_queue (
    job_id TEXT PRIMARY KEY,
    target_thread_id TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    owner_user_id INTEGER,
    discord_message_id INTEGER,
    prompt TEXT NOT NULL,
    queued INTEGER NOT NULL,
    ack_sent INTEGER NOT NULL,
    state TEXT NOT NULL,
    attempt_count INTEGER NOT NULL,
    turn_id TEXT,
    baseline_turn_ids TEXT NOT NULL,
    last_error TEXT NOT NULL DEFAULT '',
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    app_server_generation INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE deferred_discord_inbox (
    message_id INTEGER PRIMARY KEY,
    target_thread_id TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    owner_user_id INTEGER,
    prompt TEXT NOT NULL,
    source TEXT NOT NULL,
    normalization_version INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('received', 'promoted', 'completed', 'failed', 'cancelled', 'needs_review')),
    queue_job_id TEXT UNIQUE,
    promotion_epoch INTEGER,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL
);

CREATE TABLE deferred_discord_inbox_leases (
    target_thread_id TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    lease_owner TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    lease_expires_at REAL NOT NULL,
    updated_at REAL NOT NULL,
    PRIMARY KEY(target_thread_id, channel_id)
);

CREATE TABLE discord_processed_messages (
    message_id INTEGER PRIMARY KEY,
    seen_at REAL NOT NULL
);

CREATE TABLE gpt_chat_creation_ops (
    codex_thread_id TEXT PRIMARY KEY NOT NULL,
    project_key TEXT NOT NULL CHECK(project_key = 'codex:chats'),
    thread_title TEXT NOT NULL,
    discord_parent_channel_id INTEGER NOT NULL,
    nonce TEXT NOT NULL UNIQUE CHECK(length(nonce) = 32 AND nonce NOT GLOB '*[^0-9a-f]*'),
    status TEXT NOT NULL CHECK(status IN ('prepared', 'create_started', 'discord_identified')),
    discord_thread_id INTEGER,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL
);

CREATE TABLE mirror_projects (
    project_key TEXT PRIMARY KEY,
    project_name TEXT NOT NULL,
    discord_channel_id INTEGER NOT NULL,
    updated_at REAL NOT NULL
);

CREATE TABLE mirror_threads (
    codex_thread_id TEXT PRIMARY KEY,
    project_key TEXT NOT NULL,
    thread_title TEXT NOT NULL,
    discord_channel_id INTEGER NOT NULL,
    discord_thread_id INTEGER NOT NULL,
    updated_at REAL NOT NULL,
    managed_by TEXT NOT NULL DEFAULT 'ordinary' CHECK(managed_by IN ('ordinary', 'gpt_chat')),
    lifecycle_state TEXT NOT NULL DEFAULT 'active' CHECK(lifecycle_state IN ('active', 'deactivating', 'inactive', 'reactivating')),
    detail_mode TEXT NOT NULL DEFAULT 'send' CHECK(detail_mode IN ('send', 'all'))
);

CREATE TABLE persistent_component_claims (
    claim_key TEXT PRIMARY KEY,
    created_at REAL NOT NULL,
    expires_at REAL NOT NULL
);

CREATE TABLE session_mirror_details (
    codex_thread_id TEXT PRIMARY KEY,
    detail_mode TEXT NOT NULL CHECK(detail_mode IN ('send', 'all'))
);

CREATE INDEX codex_turn_attempts_job_order
    ON codex_turn_attempts(job_id, attempt_number DESC);

CREATE UNIQUE INDEX codex_turn_queue_message_id
    ON codex_turn_queue(discord_message_id)
    WHERE discord_message_id IS NOT NULL;

CREATE INDEX codex_turn_queue_target_order
    ON codex_turn_queue(target_thread_id, created_at, job_id);

CREATE INDEX deferred_discord_inbox_target_order
    ON deferred_discord_inbox(target_thread_id, channel_id, state, created_at, message_id);
