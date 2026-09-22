//! Whether the login keyring is wired into PAM. See HuginnKeyring's
//! `docs/pam.md` and `huginn-keyring --install-pam` / `--remove-pam`.
//!
//! Unlike fingerprint sudo (`backend::fingerprint`) this asks nothing of
//! `ravend` and keeps no per-account policy: the two PAM lines are
//! `optional`, safe to add unconditionally, never denied and never widened,
//! so there is nothing here for a socket to answer and nothing to confirm
//! with a password. The two lines being in the file *is* the whole of the
//! state, so this module is a file read and a command line, nothing more.
//!
//! `wired()`'s check is mirrored from `huginn-keyring`'s own `pam::has_line`
//! rather than depended on, the same way `fingerprint::sudo_wired` mirrors
//! `raven-finger-auth`'s: this repository builds on its own, and a settings
//! panel has no business linking a keyring daemon's crate to ask one question
//! a `read_to_string` already answers.

use std::path::Path;

/// What Settings offers to run. Ships with HuginnKeyring.
pub const HELPER: &str = "/usr/bin/huginn-keyring";

const PAM_FILE: &str = "/etc/pam.d/system-login";

/// The exact two lines `huginn-keyring --install-pam` writes.
const MARKER: &str = "pam_huginn_keyring.so";
const GROUPS: [&str; 2] = ["session", "password"];

/// Whether `huginn-keyring` is installed at all.
pub fn helper_installed() -> bool {
    Path::new(HELPER).exists()
}

/// Whether both lines are in `/etc/pam.d/system-login` already. `false` if
/// either is missing, since `--install-pam` would still change something --
/// the two are meant to travel together.
pub fn wired() -> bool {
    let Ok(text) = std::fs::read_to_string(PAM_FILE) else {
        return false;
    };
    GROUPS.iter().all(|group| {
        text.lines().any(|line| {
            let line = line.trim_start();
            !line.starts_with('#')
                && line
                    .split_whitespace()
                    .next()
                    .is_some_and(|ty| ty.trim_start_matches('-') == *group)
                && line.contains(MARKER)
        })
    })
}

/// The command that adds the lines, for the user's terminal.
pub fn install_command() -> Vec<String> {
    vec!["sudo".into(), HELPER.into(), "install-pam".into()]
}

/// The command that removes them.
pub fn remove_command() -> Vec<String> {
    vec!["sudo".into(), HELPER.into(), "remove-pam".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same fixture `huginn-keyring`'s own tests use, and the same claim:
    /// the two modules must agree on what "wired" means, or this page could
    /// show a switch on that `huginn-keyring --pam-status` would call off.
    #[test]
    fn install_and_remove_commands_name_the_same_helper() {
        assert_eq!(install_command()[1], HELPER);
        assert_eq!(remove_command()[1], HELPER);
        assert_eq!(install_command()[2], "install-pam");
        assert_eq!(remove_command()[2], "remove-pam");
    }
}
