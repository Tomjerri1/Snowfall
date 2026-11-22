//! HTTPS GET through the system `curl`, for Unix platforms that have no built-in HTTP client.
//!
//! Spawning `curl` (preinstalled on macOS and virtually every Linux desktop) avoids bundling a TLS stack
//! for a single update check request. Arguments are passed directly, never through a shell.

use std::process::{Command, Stdio};

pub fn http_get(host: &str, path: &str) -> Result<String, String> {
    let url = format!("https://{}{}", host, path);
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-time",
            "10",
            "--header",
            "Accept: application/vnd.github+json",
            "--user-agent",
            "Snowfall-update-check",
            "--",
            &url,
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run curl: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("curl failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
