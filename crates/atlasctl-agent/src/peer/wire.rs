// SPDX-License-Identifier: AGPL-3.0-only

//! Frames on the agent-to-agent channel, and how they cross it.
//!
//! A **separate enum from `ClientMsg`**, on purpose. The browser surface and the
//! peer surface have different threat models and different authentication, and
//! sharing one enum between them is how a verb intended for a paired agent ends
//! up reachable from a web page. Keeping them apart makes that mistake require
//! writing the verb twice.
//!
//! Nothing here carries a command. The launch frame names a recipe and a rank;
//! the receiving agent renders its own docker command from its own vendored
//! copy. That is what bounds the damage a compromised head can do.

use crate::cluster::{PrepareReply, RankAssignment};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest frame accepted, so a peer cannot ask us to allocate without bound.
const MAX_FRAME: u32 = 1 << 20;

/// What one agent says to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PeerFrame {
    /// Opening frame: who I am, and what I can do.
    Hello {
        /// Protocol version.
        version: u32,
        /// Display name, for the other side's interface.
        name: String,
        /// Whether this node can run a model.
        can_launch: bool,
        /// Coarse accelerator tag.
        accelerator: String,
    },

    /// Begin a pairing exchange. Carries the initiator's SPAKE2 message.
    PairStart {
        /// SPAKE2 message bytes, hex encoded.
        message: String,
    },

    /// Answer a pairing exchange with this side's SPAKE2 message.
    PairAnswer {
        /// SPAKE2 message bytes, hex encoded.
        message: String,
    },

    /// Key confirmation. Both sides send one; both verify the other's.
    PairConfirm {
        /// MAC over the agreed key and the TLS channel binding.
        mac: String,
    },

    /// Pairing failed. Sent so the other side stops waiting rather than timing
    /// out, and so the reason can be shown instead of guessed at.
    PairRefused {
        /// Why.
        reason: String,
    },

    /// Pairing succeeded on this side, and a pin has been written.
    PairAccepted {
        /// This node's public key, hex encoded, so the peer can pin it.
        public_key: String,
    },

    /// Render the command this rank would run, without running it.
    ///
    /// Asked of the rank itself rather than rendered by the head: the head does
    /// not know what recipe revision or hardware the other machine has, and a
    /// preview it invented would be a guess presented as the thing that will
    /// execute.
    PreviewRank {
        /// What this node would be asked to do.
        assignment: Box<RankAssignment>,
    },

    /// The command this rank would run.
    RankPreviewed {
        /// Shell-quoted, for reading and copying.
        command: String,
        /// Settings this node's flag table does not claim, so the operator can
        /// see what will silently not apply.
        unmapped: Vec<String>,
    },

    /// This rank cannot run the assignment, and why.
    RankRefused {
        /// Reason, in words the operator can act on.
        reason: String,
    },

    /// Validate and reserve for a rank. Nothing starts.
    Prepare {
        /// What this node is being asked to do.
        assignment: Box<RankAssignment>,
        /// Which prepare this belongs to.
        epoch: String,
    },

    /// The answer to a prepare.
    Prepared {
        /// Which prepare.
        epoch: String,
        /// Ready, or refused with a reason.
        reply: PrepareReply,
    },

    /// Start the rank prepared under this epoch.
    Commit {
        /// Which prepare.
        epoch: String,
    },

    /// The rank started, and this is its container.
    Committed {
        /// Which prepare.
        epoch: String,
        /// Container id on that machine.
        container: String,
    },

    /// A reservation was released. Acknowledged rather than answered with a
    /// result, so a failure to release cannot mask whatever caused the
    /// rollback that asked for it.
    Aborted {
        /// Which prepare.
        epoch: String,
    },

    /// Release a reservation without starting anything.
    Abort {
        /// Which prepare.
        epoch: String,
    },

    /// Stop a container this peer started as a rank.
    ///
    /// Names a container rather than a recipe, so a head can only stop what it
    /// was told about — not an unrelated workload the operator is running.
    StopRank {
        /// Container id, as returned by the commit that started it.
        container: String,
    },

    /// The rank was stopped, or was already not running.
    RankStopped {
        /// Which container.
        container: String,
    },

    /// Periodic vitals, once paired.
    Vitals {
        /// The sample.
        vitals: Box<atlasctl_protocol::fleet::NodeVitals>,
    },
}

/// Version of the peer protocol this build speaks.
pub const PEER_PROTOCOL_VERSION: u32 = 1;

/// Write one frame, length-prefixed.
///
/// # Errors
/// If the frame cannot be serialised or the socket rejects the write.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &PeerFrame) -> Result<()> {
    let body = serde_json::to_vec(frame).context("serialising a peer frame")?;
    let len = u32::try_from(body.len()).context("frame is absurdly large")?;
    anyhow::ensure!(len <= MAX_FRAME, "refusing to send a {len}-byte frame");
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

/// Read one frame.
///
/// The length is checked before anything is allocated, so a peer cannot make
/// this process reserve a gigabyte by claiming it is about to send one.
///
/// # Errors
/// If the stream ends, the length is implausible, or the body is not a frame.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<PeerFrame> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .await
        .context("reading a peer frame length")?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        bail!("peer announced a {len}-byte frame; the limit is {MAX_FRAME}");
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)
        .await
        .context("reading a peer frame body")?;
    serde_json::from_slice(&body).context("decoding a peer frame")
}
