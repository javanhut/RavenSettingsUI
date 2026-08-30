//! Small helpers shared by the backends.

use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

/// Run a command to completion and return its stdout, failing on a non-zero
/// exit with stderr in the message.
pub fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() {
            format!("{program} exited with {}", out.status)
        } else {
            err
        };
        return Err(anyhow!(msg));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run a command and return stdout and stderr together, in that order,
/// regardless of exit status. For tools that print their report on stderr.
pub fn run_all(program: &str, args: &[&str]) -> Result<(bool, String)> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("could not run {program}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// Whether `program` is on PATH.
pub fn have(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Human-readable size, binary units.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(3 << 30), "3.0 GiB");
    }
}
