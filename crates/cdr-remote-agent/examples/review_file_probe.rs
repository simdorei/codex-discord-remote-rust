//! Read-only local evidence probe using the same public file boundary as MCP.
use cdr_remote_agent::files::ProjectFileAccess;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("project root required")?;
    let access = ProjectFileAccess::open(root)?;
    let mut reports = Vec::new();
    for path in args {
        let result = access.read_file(&path, 1, 500)?;
        let masked_lines: Vec<usize> = result
            .content
            .lines()
            .enumerate()
            .filter_map(|(index, line)| line.contains("[REDACTED]").then_some(index + 1))
            .collect();
        reports.push(serde_json::json!({
            "path": path, "sha256": result.sha256, "redacted": result.redacted,
            "masked_lines": masked_lines, "end_line": result.end_line,
            "total_lines": result.total_lines, "truncated": result.truncated,
        }));
    }
    println!("{}", serde_json::to_string_pretty(&reports)?);
    Ok(())
}
