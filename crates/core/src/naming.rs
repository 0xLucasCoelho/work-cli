//! Naming conventions for work resources. PURE: no IO.

/// Reserved tokens — equal to a CLI verb, so they cannot also be a workspace name.
///
/// Single source of truth for the CLI's verbs. `work-cli`'s `main::RESERVED`
/// references this set so the two can never drift. Any new CLI verb MUST be
/// added here (and `validate_name` will then reject it as a workspace name).
pub const RESERVED: &[&str] = &[
    "new", "all", "browse", "ls", "start", "stop", "stop-all", "resume", "fwd", "config", "image",
    "doctor", "help", "version", "rm", "tab", "tabs", "update",
];

pub fn volume(ws: &str) -> String {
    format!("work-{ws}-home")
}

pub fn network(ws: &str) -> String {
    format!("work-net-{ws}")
}

pub fn container(ws: &str) -> String {
    format!("work-{ws}")
}

/// Validate a workspace name. Lowercase `[a-z0-9][a-z0-9-]*`, length 1..=40, not reserved.
pub fn validate_name(name: &str) -> Result<(), crate::error::NameError> {
    use crate::error::NameError;
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > 40 {
        return Err(NameError::TooLong);
    }
    let mut chars = name.chars();
    let first = chars.next().expect("non-empty");
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(NameError::InvalidChar);
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(NameError::InvalidChar);
    }
    if RESERVED.contains(&name) {
        return Err(NameError::Reserved);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_includes_every_cli_verb() {
        for verb in [
            "image", "doctor", "help", "version", "rm", "tab", "tabs", "update",
        ] {
            assert!(RESERVED.contains(&verb), "missing reserved verb: {verb}");
            assert!(
                matches!(validate_name(verb), Err(crate::error::NameError::Reserved)),
                "verb not rejected by validate_name: {verb}"
            );
        }
    }
}
