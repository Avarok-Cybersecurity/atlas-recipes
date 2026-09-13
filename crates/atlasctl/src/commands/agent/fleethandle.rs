// SPDX-License-Identifier: AGPL-3.0-only

//! The fleet handle `agent run` hands to the server and the daemon loops.

use std::sync::Arc;

/// Lets the background loops and the server share one fleet.
///
/// `AgentState` wants an owned `Box<dyn FleetView>` while the daemon loops need
/// an `Arc`; this forwards rather than duplicating the state, so a peer
/// discovered by the loops is visible to the next browser request.
pub(super) struct FleetHandle(pub(super) Arc<atlasctl_agent::fleet::LocalFleet>);

impl atlasctl_agent::fleet::FleetView for FleetHandle {
    fn nodes(&self) -> Vec<atlasctl_protocol::fleet::NodeDescriptor> {
        self.0.nodes()
    }

    fn pair<'a>(
        &'a self,
        node: atlasctl_protocol::fleet::NodeId,
        code: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<atlasctl_agent::fleet::PairOutcome>>
                + Send
                + 'a,
        >,
    > {
        self.0.pair(node, code)
    }

    fn pair_at<'a>(
        &'a self,
        target: &'a str,
        code: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<atlasctl_agent::fleet::PairOutcome>>
                + Send
                + 'a,
        >,
    > {
        self.0.pair_at(target, code)
    }

    fn trust(
        &self,
        outcome: &atlasctl_agent::fleet::PairOutcome,
        allow_control: bool,
    ) -> anyhow::Result<()> {
        self.0.trust(outcome, allow_control)
    }

    fn unpair(&self, node: atlasctl_protocol::fleet::NodeId) -> anyhow::Result<bool> {
        self.0.unpair(node)
    }
}
