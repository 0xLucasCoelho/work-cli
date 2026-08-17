//! Naming conventions for work resources. PURE: no IO.

use crate::error::NameError;

/// Reserved tokens — equal to a CLI verb, so they cannot also be a workspace name.
pub const RESERVED: &[&str] = &[
    "new", "ls", "start", "stop", "rm", "doctor", "attach", "image", "help", "version", "browse",
    "fwd", "config", "app", "migrate", "harden",
];

/// OCI label key marking an object as created and owned by `work`.
pub const LABEL_KEY: &str = "dev.work-cli.managed";

pub const HOME_TARGET: &str = "/home/dev";

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
pub fn validate_name(name: &str) -> Result<(), NameError> {
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
    if name.starts_with("fwd-") || name.starts_with("browse-") {
        return Err(NameError::ReservedPrefix);
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
    fn reserved_includes_core_verbs() {
        for verb in ["new", "ls", "start", "stop", "rm", "doctor", "attach", "app"] {
            assert!(RESERVED.contains(&verb), "missing reserved verb: {verb}");
            assert!(
                matches!(validate_name(verb), Err(NameError::Reserved)),
                "verb not rejected: {verb}"
            );
        }
    }

    #[test]
    fn rejects_forwarder_prefixes() {
        for name in ["fwd-acme-8080", "browse-acme-3000", "fwd-x"] {
            assert!(matches!(
                validate_name(name),
                Err(NameError::ReservedPrefix)
            ));
        }
        assert!(validate_name("acme-fwd").is_ok());
    }

    #[test]
    fn accepts_simple_names() {
        assert!(validate_name("acme").is_ok());
        assert!(validate_name("shop-vision").is_ok());
    }
}
