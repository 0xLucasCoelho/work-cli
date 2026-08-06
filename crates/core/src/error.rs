use std::fmt;

/// Error returned by workspace-name validation. Kept as its own type so the
/// CLI can map it to a friendly "invalid name" message distinct from IO errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    InvalidChar,
    Reserved,
    ReservedPrefix,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NameError::Empty => write!(f, "name must not be empty"),
            NameError::TooLong => write!(f, "name must be at most 40 characters"),
            NameError::InvalidChar => {
                write!(f, "name must match [a-z0-9][a-z0-9-]* (lowercase)")
            }
            NameError::Reserved => write!(f, "name is reserved (matches a command)"),
            NameError::ReservedPrefix => write!(
                f,
                "name uses a reserved prefix (fwd-/browse- are forwarder container names)"
            ),
        }
    }
}

impl std::error::Error for NameError {}
