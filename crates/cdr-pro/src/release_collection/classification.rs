use super::{Outcome, Status};

#[must_use]
pub fn classify_test(outcome: &Outcome) -> Status {
    classify(outcome, None)
}

pub(super) fn classify_required_targets(outcome: &Outcome, suites: usize) -> Status {
    classify(outcome, Some(suites))
}

fn classify(outcome: &Outcome, expected_suites: Option<usize>) -> Status {
    if outcome.code != 0 {
        return Status::Failed;
    }
    let output = format!("{}\n{}", outcome.stdout, outcome.stderr).to_lowercase();
    if output.contains("skipped=") || output.contains(" skipped") {
        return Status::Skipped;
    }
    let pattern = regex::Regex::new(
        r"^test result: (ok|failed)\. (\d+) passed; (\d+) failed; (\d+) ignored(?:;|$)",
    )
    .expect("constant regex");
    let mut suites = 0;
    let mut skipped = false;
    for line in output.lines().map(str::trim) {
        if !line.starts_with("test result:") {
            continue;
        }
        let Some(captures) = pattern.captures(line) else {
            return Status::Malformed;
        };
        let (Ok(passed), Ok(failed), Ok(ignored)) = (
            captures[2].parse::<u64>(),
            captures[3].parse::<u64>(),
            captures[4].parse::<u64>(),
        ) else {
            return Status::Malformed;
        };
        if &captures[1] != "ok" || failed != 0 {
            return Status::Failed;
        }
        suites += 1;
        skipped |= ignored != 0 || passed == 0;
    }
    if suites == 0 || skipped {
        Status::Skipped
    } else if expected_suites.is_some_and(|expected| expected != suites) {
        Status::Malformed
    } else {
        Status::Passed
    }
}
