// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn iface(name: &str, addrs: &[&str], carrier: bool, speed: Option<u32>) -> RawIface {
    RawIface {
        name: name.to_owned(),
        addrs: addrs.iter().map(|s| (*s).to_owned()).collect(),
        carrier,
        speed_mbps: speed,
        arp_type: 1,
        wireless: false,
        rdma: false,
    }
}

/// The interface table of a real DGX Spark, read off the box on 2026-08-25.
///
/// Kept verbatim rather than simplified, because every awkward part of it is
/// load-bearing: two docker bridges that are UP, a `dummy0` carrying the
/// documented head IP 10.10.10.1, wifi that is the only generally routable
/// link, and four RoCE ports on point-to-point /30s.
fn real_dgx_spark() -> Vec<RawIface> {
    let roce = |name: &str, addr: &str| RawIface {
        rdma: true,
        ..iface(name, &[addr], true, Some(200_000))
    };
    vec![
        RawIface {
            arp_type: 772,
            ..iface("lo", &["127.0.0.1"], true, None)
        },
        iface("docker0", &["172.17.0.1"], false, None),
        iface("docker_gwbridge", &["172.19.0.1"], true, Some(10_000)),
        iface("veth188ed8c", &[], true, Some(10_000)),
        iface("dummy0", &["10.10.10.1"], true, None),
        RawIface {
            wireless: true,
            ..iface("wlP9s9", &["192.168.68.68"], true, None)
        },
        roce("enp1s0f0np0", "10.10.10.9"),
        roce("enp1s0f1np1", "10.10.10.13"),
        iface("enP7s7", &[], false, None),
    ]
}

#[test]
fn the_real_spark_table_selects_roce_and_nothing_else() {
    let addrs = usable_addresses(&real_dgx_spark());

    // Both RoCE ports survive, best-first, and nothing else does.
    let names: Vec<&str> = addrs.iter().map(|a| a.iface.as_str()).collect();
    assert_eq!(names, ["enp1s0f0np0", "enp1s0f1np1"]);
    assert!(addrs.iter().all(|a| a.class == LinkClass::Roce));
    assert!(addrs.iter().all(|a| a.rdma));

    // The specific traps, stated so a regression names itself.
    assert!(
        !names.contains(&"docker_gwbridge"),
        "an UP docker bridge must never be offered as a cluster address"
    );
    assert!(
        !names.contains(&"dummy0"),
        "10.10.10.1 lives on a dummy interface: it looks authoritative and routes nowhere"
    );
    assert!(
        !names.contains(&"wlP9s9"),
        "wifi is the only routable link on this box, and is still not a fabric"
    );
    assert!(!names.contains(&"lo"), "loopback is not a cluster address");
}

#[test]
fn classification_is_by_what_the_link_is_not_what_it_is_called() {
    let t = real_dgx_spark();
    let of = |n: &str| classify(t.iter().find(|i| i.name == n).expect("fixture has it"));

    assert_eq!(of("lo"), LinkClass::Loopback);
    assert_eq!(of("docker0"), LinkClass::Virtual);
    assert_eq!(of("docker_gwbridge"), LinkClass::Virtual);
    assert_eq!(of("veth188ed8c"), LinkClass::Virtual);
    assert_eq!(of("dummy0"), LinkClass::Virtual);
    assert_eq!(of("wlP9s9"), LinkClass::Wireless);
    assert_eq!(of("enp1s0f0np0"), LinkClass::Roce);
    assert_eq!(of("enP7s7"), LinkClass::Ethernet);
}

#[test]
fn an_interface_with_no_carrier_is_not_offered() {
    // enP7s7 and docker0 are both DOWN on the real box.
    let addrs = usable_addresses(&[
        iface("eth0", &["10.0.0.1"], false, Some(1000)),
        iface("eth1", &["10.0.0.2"], true, Some(1000)),
    ]);
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].iface, "eth1");
}

#[test]
fn an_interface_with_a_carrier_but_no_address_is_not_offered() {
    let addrs = usable_addresses(&[iface("veth0", &[], true, Some(10_000))]);
    assert!(addrs.is_empty());
}

#[test]
fn an_amd_style_ethernet_only_box_still_forms_a_cluster_but_warns() {
    // Strix Halo has no RoCE. The state machine must still produce a usable
    // address rather than refusing, and the class must be one that warns.
    let addrs = usable_addresses(&[
        iface("enp4s0", &["192.168.1.40"], true, Some(2500)),
        iface("docker0", &["172.17.0.1"], true, None),
    ]);
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].class, LinkClass::Ethernet);
    assert!(
        addrs[0].class.warns(),
        "an ethernet-only cluster is correct but several times slower; it must warn"
    );
}

#[test]
fn ordering_is_stable_when_links_are_identical() {
    // Two identical RoCE ports must not swap places between calls, or the node
    // list in the UI reorders itself for no reason.
    let table = vec![
        RawIface {
            rdma: true,
            ..iface("enp1s0f1np1", &["10.10.10.13"], true, Some(200_000))
        },
        RawIface {
            rdma: true,
            ..iface("enp1s0f0np0", &["10.10.10.9"], true, Some(200_000))
        },
    ];
    let a = usable_addresses(&table);
    let b = usable_addresses(&table);
    assert_eq!(a, b);
    assert_eq!(a[0].iface, "enp1s0f0np0", "ties break by name, ascending");
}

#[test]
fn a_faster_link_of_the_same_class_wins() {
    let addrs = usable_addresses(&[
        iface("eth0", &["10.0.0.1"], true, Some(1000)),
        iface("eth1", &["10.0.1.1"], true, Some(25_000)),
    ]);
    assert_eq!(addrs[0].iface, "eth1");
}

#[test]
fn a_static_provider_answers_without_touching_the_system() {
    let f = StaticFabric {
        ifaces: real_dgx_spark(),
    };
    let addrs = f.addresses().expect("static provider cannot fail");
    assert_eq!(addrs[0].class, LinkClass::Roce);
}
