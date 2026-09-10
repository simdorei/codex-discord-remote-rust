use super::ActionResult;
use crate::queue_runner::{BackendFailureKind, Submission};

pub(super) fn submission_result(
    thread_id: &str,
    source: Option<&str>,
    submission: &Submission,
    raw_prompt: &str,
) -> ActionResult {
    if let Some(warning) = &submission.warning {
        let source = source.map_or_else(String::new, |value| format!("\nsource: {value}"));
        if warning.kind == BackendFailureKind::Quarantined {
            return ActionResult {
                text: format!(
                    "Codex request was not replayed{source}\
                     \nthread_id: {thread_id}\
                     \njob_id: {}\
                     \nstatus: quarantined ambiguous start\
                     \nreason: {}",
                    submission.job_id, warning.message
                ),
                waits_for_final: false,
                ui: None,
            };
        }
        if warning.kind == BackendFailureKind::ForkFenced {
            return ActionResult {
                text: format!(
                    "Codex request was not replayed{source}\
                     \nthread_id: {thread_id}\
                     \njob_id: {}\
                     \nstatus: app-server fork outcome is unresolved\
                     \nreason: {}\
                     \nsafety: duplicate fork or request replay was prevented",
                    submission.job_id, warning.message
                ),
                waits_for_final: false,
                ui: None,
            };
        }
        if warning.kind == BackendFailureKind::StartingCandidatesHeld {
            return ActionResult {
                text: format!(
                    "Codex request was not replayed{source}\
                     \nthread_id: {thread_id}\
                     \njob_id: {}\
                     \nstatus: target queue is held for manual resolution\
                     \nreason: {}\
                     \nsafety: no turn was selected and no request was replayed",
                    submission.job_id, warning.message
                ),
                waits_for_final: false,
                ui: None,
            };
        }
        let (header, warning_kind) = if warning.ambiguous {
            (
                "Accepted Codex request; immediate start outcome is unknown and recovery will reconcile it",
                "ambiguous backend failure (the start may have reached Codex)",
            )
        } else {
            (
                "Accepted Codex request; queued for automatic retry",
                "definite backend failure",
            )
        };
        return ActionResult {
            text: format!(
                "{header}{source}\
                 \nthread_id: {thread_id}\
                 \njob_id: {}\
                 \nwarning_kind: {warning_kind}\
                 \nwarning: {}",
                submission.job_id, warning.message
            ),
            waits_for_final: true,
            ui: None,
        };
    }
    eprintln!(
        "discord_request_accepted thread_id={thread_id} job_id={} queued={} turn_id={}",
        submission.job_id,
        submission.queued,
        submission.turn_id.as_deref().unwrap_or("pending")
    );
    let text = if submission.queued {
        "Queued\nmessage: 앞선 작업이 끝나면 시작합니다.".to_owned()
    } else if submission.turn_id.is_some() {
        format!("In progress\nmessage: {}", request_echo(raw_prompt))
    } else {
        "Preparing\nmessage: 작업 시작을 확인하고 있습니다.".to_owned()
    };
    ActionResult {
        text,
        waits_for_final: true,
        ui: None,
    }
}

pub(super) fn request_echo(raw_prompt: &str) -> String {
    let raw_prompt = raw_prompt
        .strip_prefix(super::format::INTERVIEW_HEADER)
        .unwrap_or(raw_prompt);
    let normalized = raw_prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let mut preview: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_runner::BackendFailure;

    #[test]
    fn echo_is_a_short_single_line_of_the_original_request() {
        assert_eq!(
            request_echo("  한글 요청\n  확인해줘  "),
            "한글 요청 확인해줘"
        );
        assert_eq!(
            request_echo(&"가".repeat(121)),
            format!("{}…", "가".repeat(120))
        );
        assert_eq!(request_echo(""), "");
        assert_eq!(
            request_echo(&format!(
                "{}한글 요청",
                crate::action_executor::format::INTERVIEW_HEADER
            )),
            "한글 요청"
        );
        assert_eq!(request_echo("User request: 원문"), "User request: 원문");
    }

    #[test]
    fn normal_start_is_short_and_waiting_is_not_reported_as_running() {
        for (queued, turn_id, expected) in [
            (false, Some("turn"), "In progress\nmessage: 요청 에코"),
            (
                true,
                None,
                "Queued\nmessage: 앞선 작업이 끝나면 시작합니다.",
            ),
            (
                false,
                None,
                "Preparing\nmessage: 작업 시작을 확인하고 있습니다.",
            ),
        ] {
            let result = submission_result(
                "thread",
                Some("mirror"),
                &Submission {
                    job_id: "job".into(),
                    queued,
                    turn_id: turn_id.map(str::to_owned),
                    warning: None,
                },
                "요청 에코",
            );
            assert_eq!(result.text, expected);
            assert!(result.waits_for_final);
        }
    }

    #[test]
    fn quarantined_duplicate_is_explicit_and_does_not_wait_for_a_final() {
        let result = submission_result(
            "forked",
            Some("mirror (app-server fork)"),
            &Submission {
                job_id: "job-a".into(),
                queued: false,
                turn_id: None,
                warning: Some(BackendFailure::quarantined("ambiguous start preserved")),
            },
            "요청 에코",
        );
        assert!(!result.waits_for_final);
        assert!(result.text.contains("was not replayed"));
        assert!(result.text.contains("quarantined"));
    }

    #[test]
    fn unresolved_fork_duplicate_is_explicit_and_does_not_wait_for_a_final() {
        let result = submission_result(
            "source",
            Some("mirror"),
            &Submission {
                job_id: "job-fenced".into(),
                queued: true,
                turn_id: None,
                warning: Some(BackendFailure::fork_fenced(
                    "thread/fork response timed out",
                )),
            },
            "요청 에코",
        );
        assert!(!result.waits_for_final);
        assert!(result.text.contains("was not replayed"));
        assert!(result.text.contains("fork outcome is unresolved"));
        assert!(result.text.contains("duplicate"));
    }

    #[test]
    fn starting_candidate_hold_is_manual_and_does_not_wait_for_a_final() {
        let result = submission_result(
            "held-thread",
            Some("mirror"),
            &Submission {
                job_id: "held-job".into(),
                queued: false,
                turn_id: None,
                warning: Some(BackendFailure::starting_candidates_held(
                    "[cdr-rust:turn-start-candidates-ambiguous:v1] candidate_count=2",
                )),
            },
            "요청 에코",
        );
        assert!(!result.waits_for_final);
        assert!(result.text.contains("was not replayed"));
        assert!(result.text.contains("target queue is held"));
        assert!(result.text.contains("manual resolution"));
        assert!(!result.text.contains("recovery will reconcile"));
        assert_eq!(
            result.text,
            "Codex request was not replayed\
             \nsource: mirror\
             \nthread_id: held-thread\
             \njob_id: held-job\
             \nstatus: target queue is held for manual resolution\
             \nreason: [cdr-rust:turn-start-candidates-ambiguous:v1] candidate_count=2\
             \nsafety: no turn was selected and no request was replayed"
        );
    }
}
