use anyhow::{Context, Result};

pub async fn tail_logs(_follow: bool, n: usize) -> Result<()> {
    let log_path = crate::config::log_file_path()?;

    if !log_path.exists() {
        println!("No daemon logs found at {}", log_path.display());
        return Ok(());
    }

    let content = std::fs::read_to_string(&log_path).context("Failed to read daemon log")?;

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);

    for line in &lines[start..] {
        println!("{}", line);
    }

    // TODO: implement follow mode with inotify
    Ok(())
}
