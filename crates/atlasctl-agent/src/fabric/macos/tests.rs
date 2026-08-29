// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::fabric::{classify, reachable_addresses};
use atlasctl_protocol::fleet::LinkClass;

/// `ifconfig -a` from an Apple-silicon MacBook on Wi-Fi.
///
/// Kept in real shape rather than trimmed to the interesting lines, because
/// the noise is what breaks parsers: option lines, `inet6` with `%zone`
/// suffixes, a bridge with double-indented configuration lines, and
/// interfaces that have no `status:` line at all.
const MACBOOK_WIFI_IFCONFIG: &str = r"lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
	options=1203<RXCSUM,TXCSUM,TXSTATUS,SW_TIMESTAMP>
	inet 127.0.0.1 netmask 0xff000000
	inet6 ::1 prefixlen 128
	inet6 fe80::1%lo0 prefixlen 64 scopeid 0x1
	nd6 options=201<PERFORMNUD,DAD>
gif0: flags=8010<POINTOPOINT,MULTICAST> mtu 1280
stf0: flags=0<> mtu 1280
anpi0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether aa:bb:cc:00:11:22
	media: none
	status: inactive
en1: flags=8963<UP,BROADCAST,SMART,RUNNING,PROMISC,SIMPLEX,MULTICAST> mtu 1500
	options=460<TSO4,TSO6,CHANNEL_IO>
	ether 36:5d:6e:00:11:22
	media: autoselect <full-duplex>
	status: inactive
ap1: flags=8862<BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	options=6460<TSO4,TSO6,CHANNEL_IO,PARTIAL_CSUM,ZEROINSERT_CSUM>
	ether f2:18:98:00:11:23
	media: autoselect
	status: inactive
en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	options=6463<RXCSUM,TXCSUM,TSO4,TSO6,CHANNEL_IO,PARTIAL_CSUM,ZEROINSERT_CSUM>
	ether f0:18:98:00:11:22
	inet6 fe80::4a1:b2c3:d4e5:f607%en0 prefixlen 64 secured scopeid 0xc
	inet 192.168.1.23 netmask 0xffffff00 broadcast 192.168.1.255
	nd6 options=201<PERFORMNUD,DAD>
	media: autoselect
	status: active
awdl0: flags=8943<UP,BROADCAST,RUNNING,PROMISC,SIMPLEX,MULTICAST> mtu 1500
	options=6460<TSO4,TSO6,CHANNEL_IO,PARTIAL_CSUM,ZEROINSERT_CSUM>
	ether 9a:bb:cc:dd:ee:ff
	inet6 fe80::98bb:ccff:fedd:eeff%awdl0 prefixlen 64 scopeid 0xd
	nd6 options=201<PERFORMNUD,DAD>
	media: autoselect
	status: active
llw0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	options=400<CHANNEL_IO>
	ether 9a:bb:cc:dd:ee:00
	inet6 fe80::98bb:ccff:fedd:ee00%llw0 prefixlen 64 scopeid 0xe
	nd6 options=201<PERFORMNUD,DAD>
	media: autoselect
	status: inactive
bridge0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	options=63<RXCSUM,TXCSUM,TSO4,TSO6>
	ether 36:5d:6e:00:11:22
	Configuration:
		id 0:0:0:0:0:0 priority 0 hellotime 0 fwddelay 0
	member: en1 flags=3<LEARNING,DISCOVER>
	media: <unknown type>
	status: inactive
utun0: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1500
	inet6 fe80::aabb:ccff:fedd:eeff%utun0 prefixlen 64 scopeid 0x10
utun1: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1380
	inet6 fe80::1122:3344:5566:7788%utun1 prefixlen 64 scopeid 0x11
utun2: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 2000
	inet6 fe80::99aa:bbcc:ddee:ff00%utun2 prefixlen 64 scopeid 0x12
utun3: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1000
	inet6 fe80::1234:5678:9abc:def0%utun3 prefixlen 64 scopeid 0x13
";

/// `networksetup -listallhardwareports` from the same MacBook.
const MACBOOK_PORTS: &str = "Hardware Port: Wi-Fi
Device: en0
Ethernet Address: f0:18:98:00:11:22

Hardware Port: Thunderbolt 1
Device: en1
Ethernet Address: 36:5d:6e:00:11:22

Hardware Port: Thunderbolt Bridge
Device: bridge0
Ethernet Address: 36:5d:6e:00:11:22

VLAN Configurations
===================
";

/// A desktop Mac with a wired adapter (en5) and Wi-Fi (en0) both up, wired on
/// a /22 so the hex-netmask conversion is exercised on a non-/24.
const WIRED_MAC_IFCONFIG: &str = r"lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
	inet 127.0.0.1 netmask 0xff000000
	inet6 ::1 prefixlen 128
en5: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	options=6467<RXCSUM,TXCSUM,VLAN_MTU,TSO4,TSO6,CHANNEL_IO,PARTIAL_CSUM,ZEROINSERT_CSUM>
	ether 00:e0:4c:00:11:22
	inet6 fe80::1cee:2dff:fe3a:4b5c%en5 prefixlen 64 secured scopeid 0x14
	inet 10.20.4.7 netmask 0xfffffc00 broadcast 10.20.7.255
	media: autoselect (1000baseT <full-duplex>)
	status: active
en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether f0:18:98:00:11:22
	inet6 fe80::4a1:b2c3:d4e5:f607%en0 prefixlen 64 secured scopeid 0xc
	inet 192.168.1.23 netmask 0xffffff00 broadcast 192.168.1.255
	media: autoselect
	status: active
";

/// `networksetup -listallhardwareports` for the wired Mac.
const WIRED_MAC_PORTS: &str = "Hardware Port: Wi-Fi
Device: en0
Ethernet Address: f0:18:98:00:11:22

Hardware Port: USB 10/100/1000 LAN
Device: en5
Ethernet Address: 00:e0:4c:00:11:22
";

fn find<'a>(raws: &'a [RawIface], name: &str) -> &'a RawIface {
    raws.iter()
        .find(|r| r.name == name)
        .expect("fixture has the interface")
}

#[test]
fn a_macbook_on_wifi_advertises_its_wifi_address() {
    // The operator failure this backend exists to fix: on macOS the agent
    // enumerated zero interfaces, so the machine advertised no address and
    // minted join invitations with an empty command bar.
    let raws = interfaces_from(MACBOOK_WIFI_IFCONFIG, MACBOOK_PORTS);
    let addrs = reachable_addresses(&raws);

    assert_eq!(
        addrs.len(),
        1,
        "exactly the Wi-Fi link is dialable; got {addrs:?}"
    );
    assert_eq!(addrs[0].iface, "en0");
    assert_eq!(addrs[0].addr, "192.168.1.23");
    assert_eq!(addrs[0].class, LinkClass::Wireless);
    assert_eq!(addrs[0].prefix_len, 24, "0xffffff00 is a /24");
    assert_eq!(
        addrs[0].speed_mbps, None,
        "macOS reports no negotiated speed; a number here would be invented"
    );
    assert!(!addrs[0].rdma);
}

#[test]
fn a_wired_mac_ranks_ethernet_above_wifi() {
    let raws = interfaces_from(WIRED_MAC_IFCONFIG, WIRED_MAC_PORTS);
    let addrs = reachable_addresses(&raws);

    let names: Vec<&str> = addrs.iter().map(|a| a.iface.as_str()).collect();
    assert_eq!(names, ["en5", "en0"], "the wire leads, Wi-Fi is fallback");
    assert_eq!(addrs[0].class, LinkClass::Ethernet);
    assert_eq!(addrs[0].prefix_len, 22, "0xfffffc00 is a /22");
    assert_eq!(addrs[1].class, LinkClass::Wireless);
}

#[test]
fn macos_names_classify_by_what_the_link_is() {
    let raws = interfaces_from(MACBOOK_WIFI_IFCONFIG, MACBOOK_PORTS);
    let of = |n: &str| classify(find(&raws, n));

    assert_eq!(of("lo0"), LinkClass::Loopback, "by the LOOPBACK flag");
    assert_eq!(of("en0"), LinkClass::Wireless, "by the Wi-Fi hardware port");
    // A bare Thunderbolt port is real hardware; it is dropped because it has
    // no address and no carrier, not by misnaming its class.
    assert_eq!(of("en1"), LinkClass::Ethernet);
    for virt in ["bridge0", "awdl0", "llw0", "ap1", "gif0", "stf0", "anpi0"] {
        assert_eq!(of(virt), LinkClass::Virtual, "{virt} is a software link");
    }
    for utun in ["utun0", "utun1", "utun2", "utun3"] {
        assert_eq!(of(utun), LinkClass::Virtual, "{utun} is a VPN endpoint");
    }
}

#[test]
fn loopback_and_software_links_are_never_offered() {
    let addrs = reachable_addresses(&interfaces_from(MACBOOK_WIFI_IFCONFIG, MACBOOK_PORTS));
    for a in &addrs {
        assert_eq!(
            a.iface, "en0",
            "only Wi-Fi may be offered; {} routes nowhere a peer can dial",
            a.iface
        );
    }
}

#[test]
fn ipv6_link_local_is_never_a_dialable_address() {
    let raws = interfaces_from(MACBOOK_WIFI_IFCONFIG, MACBOOK_PORTS);
    for r in &raws {
        assert!(
            r.addrs.iter().all(|a| !a.contains(':')),
            "{}: fe80:: needs a zone the peer does not have — {:?}",
            r.name,
            r.addrs
        );
    }
    // awdl0 carries ONLY a link-local address, so it must come out empty and
    // the no-address rule of the shared pipeline drops it.
    assert!(find(&raws, "awdl0").addrs.is_empty());
}

#[test]
fn carrier_is_exactly_status_active() {
    let raws = interfaces_from(MACBOOK_WIFI_IFCONFIG, MACBOOK_PORTS);
    assert!(find(&raws, "en0").carrier);
    assert!(
        !find(&raws, "en1").carrier,
        "status: inactive is no carrier"
    );
    assert!(
        !find(&raws, "utun0").carrier,
        "no status line must not present as UP"
    );
}

#[test]
fn hex_netmasks_convert_only_when_contiguous() {
    assert_eq!(prefix_from_hex_netmask("0xffffff00"), Some(24));
    assert_eq!(prefix_from_hex_netmask("0xfffffc00"), Some(22));
    assert_eq!(prefix_from_hex_netmask("0xff000000"), Some(8));
    assert_eq!(prefix_from_hex_netmask("0xffffffff"), Some(32));
    // A non-contiguous mask has no prefix length; claiming one would invent a
    // subnet. The caller falls back to the shared "unknown is zero" rule.
    assert_eq!(prefix_from_hex_netmask("0xff00ff00"), None);
    assert_eq!(prefix_from_hex_netmask("255.255.255.0"), None);
    assert_eq!(prefix_from_hex_netmask("0xnotamask"), None);
}

#[test]
fn hardware_ports_map_devices_to_port_names() {
    let ports = ports_by_device(MACBOOK_PORTS);
    assert_eq!(ports.get("en0").map(String::as_str), Some("Wi-Fi"));
    assert_eq!(ports.get("en1").map(String::as_str), Some("Thunderbolt 1"));
    assert_eq!(
        ports.get("bridge0").map(String::as_str),
        Some("Thunderbolt Bridge")
    );
    assert!(is_wireless_port("Wi-Fi"));
    assert!(is_wireless_port("AirPort"));
    assert!(!is_wireless_port("USB 10/100/1000 LAN"));
}
