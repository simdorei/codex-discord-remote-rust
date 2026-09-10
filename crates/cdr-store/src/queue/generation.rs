use std::path::Path;

use rusqlite::{TransactionBehavior, params};

use super::QueueGenerationAdoption;
use super::read::all_jobs;
use crate::Result;
use crate::schema::open_initialized;

pub fn adopt_generation(path: &Path, generation: i64) -> Result<QueueGenerationAdoption> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let adopted_count = transaction.execute(
        "UPDATE codex_turn_queue SET app_server_generation = ? \
         WHERE app_server_generation != ? AND NOT EXISTS \
         (SELECT 1 FROM codex_dead_generation_holds hold \
          WHERE hold.target_thread_id = codex_turn_queue.target_thread_id)",
        [generation, generation],
    )?;
    let jobs = all_jobs(&transaction)?;
    transaction.commit()?;
    Ok(QueueGenerationAdoption {
        jobs,
        adopted_count,
    })
}

pub fn adopt_target_generation(
    path: &Path,
    target_thread_id: &str,
    generation: i64,
) -> Result<QueueGenerationAdoption> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let adopted_count = transaction.execute(
        "UPDATE codex_turn_queue SET app_server_generation = ? \
         WHERE target_thread_id = ? AND app_server_generation != ? AND NOT EXISTS \
         (SELECT 1 FROM codex_dead_generation_holds hold \
          WHERE hold.target_thread_id = codex_turn_queue.target_thread_id)",
        params![generation, target_thread_id, generation],
    )?;
    let jobs = all_jobs(&transaction)?
        .into_iter()
        .filter(|job| job.target_thread_id == target_thread_id)
        .collect();
    transaction.commit()?;
    Ok(QueueGenerationAdoption {
        jobs,
        adopted_count,
    })
}
