//! Destructive-operation safety policy (PURE).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Kills a live session / running agents (data safe).
    WorkLoss,
    /// Irreversible data loss: volume purge (`rm --purge`).
    DataLoss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Proceed,
    Prompt,
    Refuse,
}

/// - **DataLoss** is always gated: `--yes` proceeds; else a TTY prompts; else refuse.
/// - **WorkLoss** is silent when there is nothing live to lose; otherwise gated like DataLoss.
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
            decide(Severity::DataLoss, false, true, false),
            Action::Prompt
        );
        assert_eq!(
            decide(Severity::DataLoss, false, false, true),
            Action::Proceed
        );
    }

    #[test]
    fn workloss_silent_when_no_live_session() {
        assert_eq!(
            decide(Severity::WorkLoss, false, false, false),
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
    }
}
