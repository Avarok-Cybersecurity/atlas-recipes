// SPDX-License-Identifier: AGPL-3.0-only

//! The join window: minting and revoking the code that admits one new
//! machine.
//!
//! Split from `session/tests.rs` on the 500-line cap. The seam is real: that
//! file exercises the handshake and the launch surface, this one the single
//! verb pair that can open this machine to a stranger.

use super::tests::{Fixture, TOKEN};
use super::{Session, SessionDeps};
use atlasctl_protocol::{ClientMsg, ServerMsg};

/// A session whose agent can take on a new machine.
async fn ready_with_window<'a>(f: &'a Fixture, w: &'a crate::joining::JoinWindow) -> Session<'a> {
    let (mut s, _) = Session::new(SessionDeps {
        accelerator: "",
        registry: &f.registry,
        launcher: f.launcher.clone(),
        token: TOKEN,
        can_launch: Ok(()),
        fleet: None,
        cluster: None,
        telemetry: None,
        joining: Some(w),
        relay: None,
    });
    s.handle(ClientMsg::Hello {
        protocol_version: atlasctl_protocol::PROTOCOL_VERSION,
        token: TOKEN.into(),
    })
    .await;
    s
}

#[tokio::test]
async fn minting_opens_the_window_and_hands_back_the_digits() {
    let f = Fixture::new();
    let w = crate::joining::JoinWindow::default();
    let mut s = ready_with_window(&f, &w).await;

    let out = s
        .handle(ClientMsg::MintJoinCode {
            id: 1,
            allow_control: false,
        })
        .await;
    match &out[0] {
        ServerMsg::JoinInvitation {
            code, expires_in_s, ..
        } => {
            let code = code.as_deref().expect("a code");
            assert!(crate::joining::looks_like_code(code), "{code}");
            assert_eq!(*expires_in_s, crate::joining::JOIN_TTL.as_secs());
        }
        other => panic!("expected an invitation, got {other:?}"),
    }
    assert!(w.is_open(), "the listener must now admit a joining machine");
}

/// Revoking is how an operator shuts a window they opened by mistake, and the
/// absent code is how the page knows it is shut.
#[tokio::test]
async fn revoking_closes_the_window_and_says_so() {
    let f = Fixture::new();
    let w = crate::joining::JoinWindow::default();
    let mut s = ready_with_window(&f, &w).await;
    s.handle(ClientMsg::MintJoinCode {
        id: 1,
        allow_control: false,
    })
    .await;

    let out = s.handle(ClientMsg::RevokeJoinCode { id: 2 }).await;
    match &out[0] {
        ServerMsg::JoinInvitation { code, .. } => assert!(code.is_none()),
        other => panic!("expected an invitation, got {other:?}"),
    }
    assert!(!w.is_open());
}

/// An agent with no window must not mint a code nothing will honour — that
/// would send the operator to another machine to run a command that fails.
#[tokio::test]
async fn an_agent_that_cannot_take_members_refuses_to_mint() {
    let f = Fixture::new();
    let mut s = f.ready().await;
    let out = s
        .handle(ClientMsg::MintJoinCode {
            id: 1,
            allow_control: false,
        })
        .await;
    assert!(
        matches!(out[0], ServerMsg::Error { .. }),
        "expected a refusal, got {out:?}"
    );
}

/// Minting is a Ready-phase verb: an unauthenticated socket must not be able
/// to open this machine to a stranger.
#[tokio::test]
async fn minting_before_the_handshake_is_refused() {
    let f = Fixture::new();
    let w = crate::joining::JoinWindow::default();
    let (mut s, _) = Session::new(SessionDeps {
        accelerator: "",
        registry: &f.registry,
        launcher: f.launcher.clone(),
        token: TOKEN,
        can_launch: Ok(()),
        fleet: None,
        cluster: None,
        telemetry: None,
        joining: Some(&w),
        relay: None,
    });
    let out = s
        .handle(ClientMsg::MintJoinCode {
            id: 1,
            allow_control: false,
        })
        .await;
    assert!(
        matches!(out[0], ServerMsg::Error { .. }),
        "expected a refusal, got {out:?}"
    );
    assert!(
        !w.is_open(),
        "an unauthenticated caller must not be able to open the window"
    );
}
