//! Pure fail-closed authorization logic for a future locked-use component.
//!
//! This crate cannot unlock a Mac and deliberately has no password storage,
//! Authorization Plug-in loader, display-capture backend or general-purpose
//! unlock API. It only evaluates whether a privileged host may begin its own
//! narrowly scoped authorization flow.

use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const MAX_AUTHORIZATION_WINDOW: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub bundle_id: String,
    pub bundle_path: PathBuf,
    pub team_id: String,
}

#[derive(Clone, Debug)]
pub struct TrustedTurn {
    pub id: String,
    pub issued_at: Instant,
    pub expires_at: Instant,
}

#[derive(Clone, Debug)]
pub struct Evidence {
    pub turn: Option<TrustedTurn>,
    pub requester: Option<Identity>,
    pub expected: Identity,
    pub plugin_trusted: bool,
    pub display_cover_ready: bool,
    pub local_input_seen: bool,
    pub now: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    PluginUntrusted,
    NoActiveTurn,
    TurnExpired,
    AuthorizationWindowTooLong,
    IdentityUnavailable,
    IdentityMismatch,
    LocalInputObserved,
    DisplayCoverUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The privileged host may start its own short-lived authorization flow.
    /// This does not itself unlock anything.
    BeginAuthorization {
        until: Instant,
    },
    Deny(DenyReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalInputAction {
    RelockImmediately,
    PauseAutomaticUnlock,
}

pub fn evaluate(evidence: &Evidence) -> Decision {
    if !evidence.plugin_trusted {
        return Decision::Deny(DenyReason::PluginUntrusted);
    }
    let Some(turn) = evidence.turn.as_ref() else {
        return Decision::Deny(DenyReason::NoActiveTurn);
    };
    let Some(requester) = evidence.requester.as_ref() else {
        return Decision::Deny(DenyReason::IdentityUnavailable);
    };
    if requester != &evidence.expected {
        return Decision::Deny(DenyReason::IdentityMismatch);
    }
    if turn.expires_at <= evidence.now {
        return Decision::Deny(DenyReason::TurnExpired);
    }
    if turn.expires_at.saturating_duration_since(turn.issued_at) > MAX_AUTHORIZATION_WINDOW {
        return Decision::Deny(DenyReason::AuthorizationWindowTooLong);
    }
    if evidence.local_input_seen {
        return Decision::Deny(DenyReason::LocalInputObserved);
    }
    if !evidence.display_cover_ready {
        return Decision::Deny(DenyReason::DisplayCoverUnavailable);
    }
    Decision::BeginAuthorization {
        until: turn.expires_at,
    }
}

pub fn on_local_input(already_unlocked: bool) -> Option<LocalInputAction> {
    already_unlocked.then_some(LocalInputAction::RelockImmediately)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            bundle_id: "dev.oxide.computer-use.locked-use".into(),
            bundle_path: "/Applications/Oxide Locked Use.app".into(),
            team_id: "TEAM".into(),
        }
    }

    fn evidence(now: Instant) -> Evidence {
        Evidence {
            turn: Some(TrustedTurn {
                id: "turn".into(),
                issued_at: now,
                expires_at: now + Duration::from_secs(10),
            }),
            requester: Some(identity()),
            expected: identity(),
            plugin_trusted: true,
            display_cover_ready: true,
            local_input_seen: false,
            now,
        }
    }

    #[test]
    fn complete_evidence_only_begins_a_short_authorization_flow() {
        let now = Instant::now();
        assert_eq!(
            evaluate(&evidence(now)),
            Decision::BeginAuthorization {
                until: now + Duration::from_secs(10)
            }
        );
    }

    #[test]
    fn every_missing_or_stale_condition_denies() {
        let now = Instant::now();
        let mut item = evidence(now);
        item.plugin_trusted = false;
        assert_eq!(evaluate(&item), Decision::Deny(DenyReason::PluginUntrusted));

        let mut item = evidence(now);
        item.turn = None;
        assert_eq!(evaluate(&item), Decision::Deny(DenyReason::NoActiveTurn));

        let mut item = evidence(now);
        item.requester = None;
        assert_eq!(
            evaluate(&item),
            Decision::Deny(DenyReason::IdentityUnavailable)
        );

        let mut item = evidence(now);
        item.requester.as_mut().unwrap().team_id = "OTHER".into();
        assert_eq!(
            evaluate(&item),
            Decision::Deny(DenyReason::IdentityMismatch)
        );

        let mut item = evidence(now);
        item.local_input_seen = true;
        assert_eq!(
            evaluate(&item),
            Decision::Deny(DenyReason::LocalInputObserved)
        );

        let mut item = evidence(now);
        item.display_cover_ready = false;
        assert_eq!(
            evaluate(&item),
            Decision::Deny(DenyReason::DisplayCoverUnavailable)
        );
    }

    #[test]
    fn stale_or_overlong_turns_are_refused() {
        let now = Instant::now();
        let mut item = evidence(now);
        item.now = now + Duration::from_secs(11);
        assert_eq!(evaluate(&item), Decision::Deny(DenyReason::TurnExpired));

        let mut item = evidence(now);
        item.turn.as_mut().unwrap().expires_at =
            now + MAX_AUTHORIZATION_WINDOW + Duration::from_secs(1);
        assert_eq!(
            evaluate(&item),
            Decision::Deny(DenyReason::AuthorizationWindowTooLong)
        );
    }

    #[test]
    fn local_input_always_requires_relock_while_unlocked() {
        assert_eq!(
            on_local_input(true),
            Some(LocalInputAction::RelockImmediately)
        );
        assert_eq!(on_local_input(false), None);
    }
}
