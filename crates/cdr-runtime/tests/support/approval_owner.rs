pub fn running(db: &std::path::Path, generation: u64) {
    let generation = i64::try_from(generation).unwrap();
    cdr_store::queue::enqueue(
        db,
        cdr_store::queue::NewQueueJob {
            job_id: "owner",
            target_thread_id: "thread-b",
            channel_id: 42,
            owner_user_id: Some(3),
            discord_message_id: Some(100),
            app_server_generation: generation,
            prompt: "original",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(db, "owner", &[], generation).unwrap();
    cdr_store::queue::mark_running(db, "owner", "turn-b", generation).unwrap();
}
