use crate::prompt_preprocessor::PromptPreprocessError;
use std::path::{Path, PathBuf};

pub(super) fn working_directory(
    state_db: &Path,
    thread_id: &str,
) -> Result<PathBuf, PromptPreprocessError> {
    let thread = cdr_codex_state::CodexThreadStore::open(state_db)
        .and_then(|store| store.load_thread(thread_id, false))
        .map_err(|error| failure(&error.to_string()))?
        .ok_or_else(|| failure("the original active thread was not found"))?;
    let path = PathBuf::from(&thread.cwd);
    if thread.cwd.trim().is_empty() || !path.is_absolute() {
        return Err(failure(
            "the original thread has no absolute project directory",
        ));
    }
    let metadata = std::fs::metadata(&path).map_err(|error| failure(&error.to_string()))?;
    if !metadata.is_dir() {
        return Err(failure("the original project path is not a directory"));
    }
    Ok(path)
}

fn failure(detail: &str) -> PromptPreprocessError {
    PromptPreprocessError {
        public_message:format!("Pro project directory could not be verified: {detail}"),
        recovery_action:"Check the original Codex thread and its project folder; no replacement project was selected.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_original_threads_keep_their_own_project_and_scope() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(include_str!("../../tests/fixtures/action_state.sql"))
            .unwrap();
        let first = temp.path().join("project-a");
        let second = temp.path().join("project-b");
        for (id, path) in [("thread-a", &first), ("thread-b", &second)] {
            std::fs::create_dir(path).unwrap();
            connection
                .execute(
                    "UPDATE threads SET cwd=?1 WHERE id=?2",
                    (path.to_str().unwrap(), id),
                )
                .unwrap();
            assert_eq!(working_directory(&db, id).unwrap(), *path);
        }
        assert_ne!(
            cdr_pro::prompt::pro_conversation_scope("thread-a"),
            cdr_pro::prompt::pro_conversation_scope("thread-b")
        );
        assert!(working_directory(&db, "missing").is_err());
        assert!(working_directory(&db, "thread-old").is_err());
        connection
            .execute("UPDATE threads SET cwd='relative' WHERE id='thread-a'", [])
            .unwrap();
        assert!(working_directory(&db, "thread-a").is_err());
    }
}
