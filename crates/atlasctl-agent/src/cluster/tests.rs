// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_protocol::fleet::{
    DisplayName, Launchability, NodeAddress, NodeDescriptor, PairingState,
};

fn addr(iface: &str, a: &str, class: LinkClass, speed: Option<u32>) -> NodeAddress {
    NodeAddress {
        iface: iface.to_owned(),
        addr: a.to_owned(),
        class,
        speed_mbps: speed,
        rdma: matches!(class, LinkClass::Roce | LinkClass::InfiniBand),
    }
}

fn node(byte: u8, name: &str, addrs: Vec<NodeAddress>) -> NodeDescriptor {
    NodeDescriptor {
        id: NodeId::from_bytes([byte; 32]),
        name: DisplayName::new(name),
        is_local: false,
        pairing: PairingState::Paired,
        addresses: addrs,
        launchability: Launchability::yes(),
        agent_version: "0.1.3".to_owned(),
        accelerator: "GB10".to_owned(),
        vitals: None,
        alerts: Vec::new(),
        running: None,
    }
}

fn roce_node(byte: u8, name: &str, a: &str) -> NodeDescriptor {
    node(
        byte,
        name,
        vec![
            addr("docker0", "172.17.0.1", LinkClass::Virtual, None),
            addr("enp1s0f0np0", a, LinkClass::Roce, Some(200_000)),
        ],
    )
}

fn settings() -> BTreeMap<String, atlasctl_protocol::settings::SettingValue> {
    BTreeMap::from([(
        "max_model_len".to_owned(),
        atlasctl_protocol::settings::SettingValue::Int(8192),
    )])
}

fn plan_two(head_byte: u8) -> Result<ClusterPlan, PlanError> {
    let a = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let b = roce_node(0xb2, "spark-43fa", "10.10.10.10");
    plan(
        "qwen3.5-122b-a10b-nvfp4-ep2",
        "sha256:abc",
        2,
        &[&a, &b],
        NodeId::from_bytes([head_byte; 32]),
        &settings(),
        "epoch-1".to_owned(),
    )
}

#[test]
fn a_two_node_plan_puts_the_head_at_rank_zero_on_its_roce_address() {
    let p = plan_two(0xb2).expect("a valid plan");
    assert_eq!(p.ranks.len(), 2);
    assert_eq!(p.ranks[0].rank, 0);
    assert_eq!(p.ranks[0].node, NodeId::from_bytes([0xb2; 32]));
    assert_eq!(p.ranks[1].rank, 1);

    // Every rank rendezvouses on the head's RoCE address, not on its docker
    // bridge and not on whatever the kernel listed first.
    for r in &p.ranks {
        assert_eq!(r.master_addr, "10.10.10.10");
        assert_eq!(r.world_size, 2);
        assert_eq!(r.master_port, DEFAULT_MASTER_PORT);
    }
    assert_eq!(p.link, LinkClass::Roce);
    assert!(
        p.link_warning.is_none(),
        "an all-RoCE cluster must not warn"
    );
}

#[test]
fn an_assignment_carries_no_command_only_a_recipe_id() {
    // The security property: a worker is told WHAT to run, never HOW. If this
    // struct ever grows a command, argv, image or env field, a compromised head
    // becomes remote code execution on every paired node.
    let p = plan_two(0xa1).expect("plan");
    let json = serde_json::to_string(&p.ranks[0]).expect("serialises");
    for forbidden in ["docker", "argv", "command", "image", "entrypoint", "env"] {
        assert!(
            !json.contains(forbidden),
            "a rank assignment must not carry `{forbidden}`: {json}"
        );
    }
    assert!(json.contains("recipe"));
    assert!(json.contains("recipe_hash"));
}

#[test]
fn the_worst_link_in_the_cluster_decides_the_warning() {
    // One ethernet hop makes the whole collective run at ethernet speed, so the
    // warning must key on the worst link, not the best.
    let fast = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let slow = node(
        0xb2,
        "spark-43fa",
        vec![addr(
            "eth0",
            "192.168.1.40",
            LinkClass::Ethernet,
            Some(2500),
        )],
    );
    let p = plan(
        "r",
        "h",
        2,
        &[&fast, &slow],
        fast.id,
        &settings(),
        "e".to_owned(),
    )
    .expect("still plannable");
    assert_eq!(p.link, LinkClass::Ethernet);
    let w = p.link_warning.expect("an ethernet cluster must warn");
    assert!(w.contains("all-reduce"), "the warning must say why: {w}");
}

#[test]
fn selecting_the_wrong_number_of_nodes_is_refused_with_both_numbers() {
    let a = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let err = plan("ep2", "h", 2, &[&a], a.id, &settings(), "e".to_owned())
        .expect_err("one node cannot satisfy a two-node recipe");
    assert_eq!(
        err,
        PlanError::NodeCount {
            recipe: "ep2".to_owned(),
            required: 2,
            selected: 1
        }
    );
    // The message has to name what to do about it.
    assert!(err.to_string().contains("needs exactly 2 nodes"));
}

#[test]
fn an_unpaired_node_cannot_be_launched_on() {
    let a = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let mut b = roce_node(0xb2, "spark-43fa", "10.10.10.10");
    b.pairing = PairingState::Discovered;
    let err = plan("r", "h", 2, &[&a, &b], a.id, &settings(), "e".to_owned())
        .expect_err("discovery is not permission");
    assert!(matches!(err, PlanError::NotPaired { .. }));
}

#[test]
fn a_control_only_node_cannot_be_given_a_rank() {
    let a = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let mut b = roce_node(0xb2, "laptop", "10.10.10.10");
    b.launchability = Launchability::no("this agent runs in --client mode");
    let err = plan("r", "h", 2, &[&a, &b], a.id, &settings(), "e".to_owned())
        .expect_err("a control node cannot host a rank");
    match err {
        PlanError::NotLaunchable { reason, .. } => assert!(reason.contains("--client")),
        other => panic!("wrong error: {other}"),
    }
}

#[test]
fn a_head_with_only_virtual_links_has_nowhere_to_rendezvous() {
    let head = node(
        0xa1,
        "spark-256a",
        vec![addr("docker0", "172.17.0.1", LinkClass::Virtual, None)],
    );
    let b = roce_node(0xb2, "spark-43fa", "10.10.10.10");
    let err = plan(
        "r",
        "h",
        2,
        &[&head, &b],
        head.id,
        &settings(),
        "e".to_owned(),
    )
    .expect_err("a docker bridge is not a rendezvous address");
    assert!(matches!(err, PlanError::NoUsableLink { .. }));
}

#[test]
fn the_head_must_be_one_of_the_selected_nodes() {
    let a = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let b = roce_node(0xb2, "spark-43fa", "10.10.10.10");
    let err = plan(
        "r",
        "h",
        2,
        &[&a, &b],
        NodeId::from_bytes([0xcc; 32]),
        &settings(),
        "e".to_owned(),
    )
    .expect_err("the head has to be in the launch");
    assert_eq!(err, PlanError::HeadNotSelected);
}

#[test]
fn one_node_cannot_take_two_ranks() {
    let a = roce_node(0xa1, "spark-256a", "10.10.10.9");
    let err = plan("r", "h", 2, &[&a, &a], a.id, &settings(), "e".to_owned())
        .expect_err("selecting the same box twice is not a cluster");
    assert_eq!(err, PlanError::DuplicateNode);
}

#[test]
fn planning_is_deterministic_so_two_planners_agree() {
    let first = plan_two(0xa1).expect("plan");
    let second = plan_two(0xa1).expect("plan");
    assert_eq!(first, second);
}

// ---- commit gate ---------------------------------------------------------

fn tracker() -> (ClusterPlan, PrepareTracker) {
    let p = plan_two(0xa1).expect("plan");
    let t = PrepareTracker::new(&p);
    (p, t)
}

#[test]
fn commit_is_refused_until_every_rank_has_actually_answered() {
    let (p, mut t) = tracker();
    assert!(!t.may_commit(), "no replies yet");
    t.record(&p.epoch, p.ranks[0].node, PrepareReply::Prepared);
    assert!(
        !t.may_commit(),
        "a missing reply is not a yes — this is what stops a partial cluster"
    );
    t.record(&p.epoch, p.ranks[1].node, PrepareReply::Prepared);
    assert!(t.may_commit());
}

#[test]
fn one_refusal_blocks_the_commit_and_names_who_holds_a_reservation() {
    let (p, mut t) = tracker();
    t.record(&p.epoch, p.ranks[0].node, PrepareReply::Prepared);
    t.record(
        &p.epoch,
        p.ranks[1].node,
        PrepareReply::Refused {
            reason: RefusalReason::AlreadyRunning("qwen3.6-27b".to_owned()).to_string(),
        },
    );
    assert!(t.complete());
    assert!(!t.may_commit());

    let refusals = t.refusals();
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].1.contains("already running"));

    // Rank 0 accepted, so it holds a reservation the caller must release.
    assert_eq!(t.prepared(), vec![p.ranks[0].node]);
}

#[test]
fn a_reply_from_a_stale_epoch_is_ignored() {
    // Otherwise a prepare from an abandoned attempt could satisfy the gate for
    // a plan that has since changed.
    let (p, mut t) = tracker();
    assert!(!t.record("some-other-epoch", p.ranks[0].node, PrepareReply::Prepared));
    assert!(!t.complete());
}

#[test]
fn a_reply_from_a_node_that_is_not_in_the_plan_is_ignored() {
    let (p, mut t) = tracker();
    assert!(!t.record(
        &p.epoch,
        NodeId::from_bytes([0xff; 32]),
        PrepareReply::Prepared
    ));
    assert!(!t.complete());
}

#[test]
fn a_recipe_that_differs_between_nodes_refuses_and_says_both_hashes() {
    // Two nodes on different atlasctl versions must not silently launch two
    // different models and call it one cluster.
    let r = RefusalReason::RecipeMismatch {
        recipe: "qwen3.5-122b-a10b-nvfp4-ep2".to_owned(),
        head: "sha256:aaa".to_owned(),
        local: "sha256:bbb".to_owned(),
    };
    let msg = r.to_string();
    assert!(msg.contains("sha256:aaa") && msg.contains("sha256:bbb"));
    assert!(
        msg.contains("same atlasctl"),
        "must say how to fix it: {msg}"
    );
}
