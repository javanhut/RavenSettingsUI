//! Privileged actions that no Raven daemon offers, run where the user can
//! see them.
//!
//! The rule (RavenLinux ARCHITECTURE.md, "Sleep" and the paragraphs after
//! it) is that privilege is a verb granted by a group on a socket owned by a
//! daemon that is already the policy gatekeeper -- raven-powerd, raven-timed,
//! rvnd, cawd -- never sudo spawned from a GUI. This process therefore never
//! runs sudo, pkexec or run0. When a page needs something none of those
//! daemons will do (start a service, edit a root-owned file, add a group), it
//! shows the exact command and, once the user agrees, opens their terminal on
//! it: sudo asks for the password there, and its output stays visible. This
//! is the pattern Raven Store uses to apply updates.

use anyhow::Result;

/// One word, quoted so `sh` reads it back verbatim.
pub fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The line a person would type to run `cmd`; also what the dialogs show.
/// Words that are plain enough are left bare so the command reads as one.
pub fn command_line(cmd: &[String]) -> String {
    cmd.iter()
        .map(|w| {
            let plain = !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./=:+,@%".contains(c));
            if plain {
                w.clone()
            } else {
                shell_quote(w)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run `cmd` in the user's terminal and return once the terminal has been
/// started; what the command does is the user's to watch. `-e` is what
/// nearly every emulator accepts; GIO's own terminal lookup is the fallback.
pub fn run(terminal: &str, cmd: &[String]) -> Result<()> {
    let script = script(cmd);
    if crate::util::have(terminal) {
        let spawned = std::process::Command::new(terminal)
            .args(["-e", "sh", "-c", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return Ok(());
        }
    }
    // GIO splits this line with g_shell_parse_argv, whose double-quote rules
    // are the POSIX ones: only these four characters need a backslash.
    let quoted = script
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    let info = gio::AppInfo::create_from_commandline(
        format!("sh -c \"{quoted}\""),
        None,
        gio::AppInfoCreateFlags::NEEDS_TERMINAL,
    )?;
    gio::prelude::AppInfoExt::launch(&info, &[], None::<&gio::AppLaunchContext>)?;
    Ok(())
}

/// What `sh -c` in the terminal runs: the command, then a pause so the
/// window does not vanish with sudo's answer still on it.
fn script(cmd: &[String]) -> String {
    format!(
        "{}; echo; echo 'Done. Press Enter to close.'; read _",
        cmd.iter()
            .map(|w| shell_quote(w))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one script whose quoting has to hold: a `sudo sh -c` whose own
    /// argument is full of quotes, wrapped once more for the terminal.
    #[test]
    fn the_terminal_script_is_sound_shell() {
        let inner = "sed -i '/^\\[lid\\]/,/^\\[/ s/^close = .*/close = \"ignore\"/' /etc/raven/power.toml && raven-rc restart powerd";
        let cmd: Vec<String> = ["sudo", "sh", "-c", inner]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = script(&cmd);
        assert!(s.starts_with("'sudo' 'sh' '-c' 'sed -i '\\''/^"));
        let ok = std::process::Command::new("sh")
            .args(["-n", "-c", &s])
            .status()
            .map(|st| st.success())
            .unwrap_or(true);
        assert!(ok, "sh -n rejected {s}");
        // Round trip: sh reads the quoted words back as the argv we built.
        let out = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "set -- {}; printf '%s\\n' \"$@\"",
                    s.split("; echo;").next().unwrap()
                ),
            ])
            .output()
            .unwrap();
        let words: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
        assert_eq!(words, cmd);
    }

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("plain"), "'plain'");
    }

    #[test]
    fn a_command_line_reads_like_one_typed() {
        let cmd: Vec<String> = ["sudo", "raven-rc", "start", "cawd"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(command_line(&cmd), "sudo raven-rc start cawd");
        let cmd: Vec<String> = ["sudo", "sh", "-c", "rm -f /x/*.png && install a 'b'"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            command_line(&cmd),
            "sudo sh -c 'rm -f /x/*.png && install a '\\''b'\\'''"
        );
    }
}
