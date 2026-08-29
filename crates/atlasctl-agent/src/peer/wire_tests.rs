// SPDX-License-Identifier: AGPL-3.0-only

//! Tests for the peer frames themselves: what still decodes across builds,
//! what can never decode at all, and what fits on the wire.

use super::wire::*;
use atlasctl_protocol::fleet::{
    DisplayName, LinkClass, MAX_VOUCHED, Metric, NodeAddress, NodeId, NodeVitals, VouchedPeer,
};
use atlasctl_protocol::msg::{ControlRep, ControlReq};

/// Length-prefix a hand-written JSON body exactly as `write_frame` would, so
/// `read_frame` sees what a real peer — of any build — would put on the wire.
fn framed(body: &str) -> Vec<u8> {
    let mut out = u32::try_from(body.len())
        .expect("test body fits")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(body.as_bytes());
    out
}

/// A frame written by a build that predates `version_max` and `vouched` must
/// still decode — the rolling-upgrade guarantee. A refusal here would
/// partition the whole fleet (vitals, cluster, everything) the day one box
/// upgrades before the other.
#[tokio::test]
async fn a_v1_hello_without_the_new_fields_still_decodes() {
    let body = r#"{"type":"hello","version":1,"name":"spark-43fa","can_launch":true,"accelerator":"GB10"}"#;
    let frame = read_frame(&mut &framed(body)[..])
        .await
        .expect("a pre-digest build's hello must decode");
    match frame {
        PeerFrame::Hello {
            version,
            version_max,
            vouched,
            ..
        } => {
            assert_eq!(version, 1);
            assert_eq!(version_max, None, "absent means 'did not say', not 0");
            assert_eq!(vouched, None, "absent means 'did not say', not 'no pins'");
        }
        other => panic!("expected a hello, got {other:?}"),
    }
}

fn full_vitals() -> NodeVitals {
    NodeVitals {
        accelerator_util: Metric::reading(87.5),
        sm_clock_mhz: Metric::reading(1_987.0),
        sm_clock_healthy_mhz: Some(1_500),
        temperature_c: Metric::reading(71.0),
        power_w: Metric::reading(94.0),
        memory_used_frac: Metric::reading(0.83),
        memory_total_bytes: Metric::reading(128_000_000_000.0),
        disk_free_bytes: Metric::reading(512_000_000_000.0),
        docker_ok: true,
        agent_uptime_s: 86_400,
    }
}

/// The worst-case digest entry: name at the display cap, several addresses,
/// full vitals. If the size bound only held for sparse entries it would not
/// be a bound.
fn fat_entry(i: usize) -> VouchedPeer {
    let mut id = [0xCDu8; 32];
    id[31] = u8::try_from(i).expect("fits");
    let addr = |iface: &str, a: &str, class| NodeAddress {
        iface: iface.to_owned(),
        addr: a.to_owned(),
        class,
        speed_mbps: Some(200_000),
        prefix_len: 30,
        rdma: true,
    };
    VouchedPeer {
        node: NodeId::from_bytes(id),
        name: DisplayName::new("spark-43fa-with-a-very-long-hostname-right-at-the-cap-xxxxxxxxx"),
        can_launch: true,
        accelerator: "NVIDIA GB10 Grace Blackwell Superchip".to_owned(),
        os: "Linux".to_owned(),
        addresses: vec![
            addr("enp1s0f0np0", "10.10.10.1", LinkClass::Roce),
            addr("enp1s0f1np1", "10.10.11.1", LinkClass::Roce),
            addr("eno1", "192.168.100.201", LinkClass::Ethernet),
            addr("wlan0", "192.168.1.44", LinkClass::Wireless),
        ],
        link: LinkClass::Roce,
        reachable: true,
        vitals: Some(full_vitals()),
        vitals_age_s: Some(4),
    }
}

/// A full digest must fit the existing frame cap with room to spare —
/// `write_frame` refuses oversized frames, so if this ever grew past
/// `MAX_FRAME` every hello between upgraded peers would fail, which is a
/// partition dressed up as a size check.
#[tokio::test]
async fn a_maximum_digest_hello_stays_under_the_frame_cap() {
    let frame = PeerFrame::Hello {
        version: PEER_PROTOCOL_VERSION,
        name: "spark-256a".to_owned(),
        can_launch: true,
        accelerator: "NVIDIA GB10".to_owned(),
        os: "Linux".to_owned(),
        addresses: Vec::new(),
        version_max: Some(PEER_PROTOCOL_MAX),
        vouched: Some((0..MAX_VOUCHED).map(fat_entry).collect()),
    };
    let mut wire = Vec::new();
    write_frame(&mut wire, &frame).await.expect("under the cap");

    // And it survives the trip whole: 64 entries out, 64 entries in.
    let back = read_frame(&mut &wire[..]).await.expect("decodes");
    match back {
        PeerFrame::Hello { vouched, .. } => {
            assert_eq!(vouched.map(|v| v.len()), Some(MAX_VOUCHED));
        }
        other => panic!("expected a hello, got {other:?}"),
    }
}

/// The no-nesting rule is a schema property, not a filter: a terminal
/// `Control` whose request is itself forward-shaped must fail to DECODE, so
/// there is no handler anywhere that could mishandle it.
#[tokio::test]
async fn a_control_frame_cannot_carry_another_hop() {
    let target = NodeId::from_bytes([7; 32]);
    let nested = format!(
        r#"{{"type":"control","req":{{"type":"control_to","node":"{target}","req":{{"type":"status"}}}}}}"#
    );
    assert!(
        read_frame(&mut &framed(&nested)[..]).await.is_err(),
        "a nested forward must be unrepresentable, not filtered"
    );
}

/// Same property for the pairing surface: `ControlReq` cannot spell a pairing
/// verb, so a relay cannot be used to run a ceremony on someone's behalf.
#[tokio::test]
async fn a_control_frame_cannot_spell_a_pairing_verb() {
    for req in [
        r#"{"type":"pair_start","message":"00ff"}"#,
        r#"{"type":"confirm_pairing","id":1}"#,
        r#"{"type":"prepare","epoch":"e1"}"#,
    ] {
        let body = format!(r#"{{"type":"control","req":{req}}}"#);
        assert!(
            read_frame(&mut &framed(&body)[..]).await.is_err(),
            "{req} must not decode as a control request"
        );
    }
}

/// The frames that do exist round-trip exactly, and `ControlTo` carries a
/// NodeId and no address — the relay resolves the target from its own state,
/// which is what makes an address-scanning proxy unrepresentable.
#[tokio::test]
async fn control_frames_round_trip() {
    let node = NodeId::from_bytes([9; 32]);
    let frames = [
        PeerFrame::ControlTo {
            node,
            req: ControlReq::Logs {
                recipe: atlasctl_protocol::id::RecipeId::parse("qwen3.8-27b").expect("id"),
                lines: 80,
            },
        },
        PeerFrame::Control {
            req: ControlReq::Status,
        },
        PeerFrame::ControlReply {
            rep: ControlRep::Refused {
                by: node,
                error: atlasctl_protocol::msg::AgentError::ControlRefused {
                    node,
                    reason: "grant control with `atlasctl peer grant-control`".to_owned(),
                },
            },
        },
    ];
    for frame in frames {
        let mut wire = Vec::new();
        write_frame(&mut wire, &frame).await.expect("writes");
        let back = read_frame(&mut &wire[..]).await.expect("reads");
        assert_eq!(back, frame);
    }
}
