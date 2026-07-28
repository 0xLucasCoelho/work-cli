//! Destructive-operation safety policy (PURE).
//!
//! The CLI supplies the TTY/`--yes` facts and applies the returned `Action`;
//! this module holds no IO so the decision table is fully unit-testable.

/// Severity of a destructive operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Kills a live session / running agents (data safe).
    /// E.g. `stop`, `rm` (default), `config --edit` recreate.
    WorkLoss,
    /// Irreversible data loss: volume purge (`rm --purge`).
    DataLoss,
}

/// What the caller should do for a gated destructive op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing to lose (or `--yes`) — proceed silently.
    Proceed,
    /// Interactive context — print a prompt and read y/N.
    Prompt,
    /// Non-interactive and not authorized — abort with an error.
    Refuse,
}

/// Decide how to handle a destructive operation.
///
/// - **DataLoss** is always gated: `--yes` proceeds; else a TTY prompts; else refuse.
/// - **WorkLoss** is silent when there is nothing live to lose; otherwise it is
///   gated exactly like DataLoss.
pub fn decide(severity: Severity, has_live_session: bool, is_tty: bool, yes: bool) -> Action {
    if yes {
        return Action::Proceed;
    }
    let gated = match severity {
        Severity::DataLoss => true,
        Severity::WorkLoss => has_live_session,
    };
    if !gated {
        Action::Proceed
    } else if is_tty {
        Action::Prompt
    } else {
        Action::Refuse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataloss_needs_yes_or_tty() {
        assert_eq!(
            decide(Severity::DataLoss, false, false, false),
            Action::Refuse
        );
        assert_eq!(
            decide(Severity::DataLoss, true, false, false),
            Action::Refuse
        );
        assert_eq!(
            decide(Severity::DataLoss, false, true, false),
            Action::Prompt
        );
        assert_eq!(
            decide(Severity::DataLoss, false, false, true),
            Action::Proceed
        );
        assert_eq!(
            decide(Severity::DataLoss, true, true, true),
            Action::Proceed
        );
    }

    #[test]
    fn workloss_silent_when_no_live_session() {
        // Nothing to lose -> proceeds silently regardless of TTY/--yes.
        assert_eq!(
            decide(Severity::WorkLoss, false, false, false),
            Action::Proceed
        );
        assert_eq!(
            decide(Severity::WorkLoss, false, true, false),
            Action::Proceed
        );
        assert_eq!(
            decide(Severity::WorkLoss, false, false, true),
            Action::Proceed
        );
    }

    #[test]
    fn workloss_with_live_session_prompts_or_refuses() {
        assert_eq!(
            decide(Severity::WorkLoss, true, true, false),
            Action::Prompt
        );
        assert_eq!(
            decide(Severity::WorkLoss, true, false, false),
            Action::Refuse
        );
        assert_eq!(
            decide(Severity::WorkLoss, true, true, true),
            Action::Proceed
        );
    }
}
