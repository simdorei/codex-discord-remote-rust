"""SQLite-backed Discord adapter persistence helpers."""

from __future__ import annotations

from pathlib import Path

from codex_discord_store_connection import connect_store

from codex_discord_store_busy import (
    claim_busy_choice_record as claim_busy_choice_record,
    claim_persistent_component_interaction as claim_persistent_component_interaction,
    cleanup_expired_busy_choices as cleanup_expired_busy_choices,
    cleanup_expired_persistent_component_claims as cleanup_expired_persistent_component_claims,
    create_busy_choice_record as create_busy_choice_record,
    get_busy_choice_counts as get_busy_choice_counts,
    get_busy_choice_record as get_busy_choice_record,
    get_persistent_component_claim_counts as get_persistent_component_claim_counts,
)
from codex_discord_store_mirror_cleanup import (
    delete_archived_mirror_state as delete_archived_mirror_state,
    delete_stale_mirror_rows as delete_stale_mirror_rows,
    get_remaining_mirror_discord_ids as get_remaining_mirror_discord_ids,
    get_stale_mirror_project_rows as get_stale_mirror_project_rows,
    get_stale_mirror_thread_rows as get_stale_mirror_thread_rows,
    is_mirrored_channel_id as is_mirrored_channel_id,
)
from codex_discord_store_mirror_map import (
    ProjectKeysMatchFunc as ProjectKeysMatchFunc,
    describe_mirrored_project_channel as describe_mirrored_project_channel,
    find_mirror_project_row_by_key as find_mirror_project_row_by_key,
    get_mirror_project_for_channel as get_mirror_project_for_channel,
    merge_mirror_project_key_aliases as merge_mirror_project_key_aliases,
    upsert_mirror_project as upsert_mirror_project,
)
from codex_discord_store_mirror_threads import (
    get_mirror_thread_row_by_codex_thread_id as get_mirror_thread_row_by_codex_thread_id,
    get_mirrored_codex_thread_id as get_mirrored_codex_thread_id,
    update_mirror_thread_discord_thread_id as update_mirror_thread_discord_thread_id,
    upsert_mirror_thread as upsert_mirror_thread,
)
from codex_discord_store_mirror_detail import (
    get_session_mirror_detail_mode as get_session_mirror_detail_mode,
    set_session_mirror_detail_mode as set_session_mirror_detail_mode,
)
from codex_discord_store_processed_messages import (
    claim_persistent_discord_message_id as claim_persistent_discord_message_id,
    cleanup_processed_discord_messages as cleanup_processed_discord_messages,
    is_processed_discord_message_id as is_processed_discord_message_id,
    mark_processed_discord_message_id as mark_processed_discord_message_id,
)
from codex_discord_store_queue import (
    QueueGenerationAdoption as QueueGenerationAdoption,
    QueueEnqueueResult as QueueEnqueueResult,
    QueueTargetForkFencedError as QueueTargetForkFencedError,
    QueueTargetStartingCandidatesHeldError as QueueTargetStartingCandidatesHeldError,
    QueueTargetMovedError as QueueTargetMovedError,
    StoredQueueJob as StoredQueueJob,
    UnresolvedForkFence as UnresolvedForkFence,
    adopt_queue_jobs_generation as adopt_queue_jobs_generation,
    begin_queue_job_attempt as begin_queue_job_attempt,
    complete_queue_job as complete_queue_job,
    discard_queue_jobs_for_generation as discard_queue_jobs_for_generation,
    discard_observed_queue_jobs as discard_observed_queue_jobs,
    enqueue_queue_job as enqueue_queue_job,
    flush_queue_jobs as flush_queue_jobs,
    get_unresolved_app_server_fork_fence as get_unresolved_app_server_fork_fence,
    get_starting_candidate_hold_for_target as get_starting_candidate_hold_for_target,
    has_replayable_queue_jobs as has_replayable_queue_jobs,
    is_protected_queue_job as is_protected_queue_job,
    is_quarantined_queue_job as is_quarantined_queue_job,
    is_quarantined_queue_values as is_quarantined_queue_values,
    is_starting_candidate_hold_job as is_starting_candidate_hold_job,
    is_starting_candidate_hold_values as is_starting_candidate_hold_values,
    list_queue_jobs as list_queue_jobs,
    mark_queue_job_running as mark_queue_job_running,
    retract_queue_job as retract_queue_job,
)
from codex_discord_store_schema import init_store_schema
from codex_discord_store_session_mirror import (
    claim_session_mirror_event as claim_session_mirror_event,
    cleanup_session_mirror_events as cleanup_session_mirror_events,
    get_or_init_session_mirror_cursor as get_or_init_session_mirror_cursor,
    get_session_mirror_offset as get_session_mirror_offset,
    get_session_mirror_targets as get_session_mirror_targets,
    has_session_mirror_event as has_session_mirror_event,
    update_session_mirror_cursor as update_session_mirror_cursor,
)
from codex_discord_store_startup_probe import (
    get_startup_probe_targets as get_startup_probe_targets,
)

def init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)
