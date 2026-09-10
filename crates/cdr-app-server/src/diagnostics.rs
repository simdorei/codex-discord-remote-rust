use std::collections::VecDeque;

const MAX_DIAGNOSTIC_LINES: usize = 512;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticSnapshot {
    pub lines: Vec<String>,
    pub retained_bytes: usize,
    pub dropped_lines: u64,
}

#[derive(Debug, Default)]
pub(crate) struct BoundedDiagnostics {
    lines: VecDeque<String>,
    retained_bytes: usize,
    dropped_lines: u64,
}

impl BoundedDiagnostics {
    pub(crate) fn push(&mut self, mut line: String) {
        if line.len() > MAX_DIAGNOSTIC_BYTES {
            line.truncate(MAX_DIAGNOSTIC_BYTES);
        }
        self.retained_bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > MAX_DIAGNOSTIC_LINES || self.retained_bytes > MAX_DIAGNOSTIC_BYTES
        {
            if let Some(removed) = self.lines.pop_front() {
                self.retained_bytes = self.retained_bytes.saturating_sub(removed.len());
                self.dropped_lines += 1;
            } else {
                break;
            }
        }
    }

    pub(crate) fn snapshot(&self) -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            lines: self.lines.iter().cloned().collect(),
            retained_bytes: self.retained_bytes,
            dropped_lines: self.dropped_lines,
        }
    }
}
