//! The systems nocterm can show, with their marks and colours.
use super::{Glyph, Os, Shape};

use Glyph::{Badge, Mark};
use Shape::{Circle, Hexagon, Shield, Square};

/// Every known system, in the order the editor offers them.
pub static CATALOG: &[Os] = &[
    Os {
        id: "ubuntu",
        name: "Ubuntu",
        color: "#E95420",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12" r="6.2" fill="none" stroke="{c}" stroke-width="2.4"/><circle cx="20" cy="12" r="2.6" fill="{c}"/><circle cx="8" cy="5.07" r="2.6" fill="{c}"/><circle cx="8" cy="18.93" r="2.6" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "debian",
        name: "Debian",
        color: "#A81D33",
        glyph: Glyph::Mark(
            r##"<path d="M16.8 7.6A7 7 0 1 0 17.4 16a5 5 0 0 1-7.6-3.6 3.2 3.2 0 0 1 5.6-2.2 1.8 1.8 0 0 1-1.6 2.8" fill="none" stroke="{c}" stroke-width="2.3" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "arch",
        name: "Arch Linux",
        color: "#1793D1",
        glyph: Glyph::Mark(
            r##"<path fill-rule="evenodd" d="M12 2 22 22c-2.8-2-6-3-10-3s-7.2 1-10 3Zm0 8-3 6.4c1-.3 2-.4 3-.4s2 .1 3 .4Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "manjaro",
        name: "Manjaro",
        color: "#35BF5C",
        glyph: Glyph::Mark(
            r##"<path d="M3 3h11v5H8.5v13H3Zm6.5 6.5H14V21H9.5ZM15.5 3H21v18h-5.5Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "alpine",
        name: "Alpine Linux",
        color: "#0D597F",
        glyph: Glyph::Mark(
            r##"<path d="M1.5 19.5 9 7l4.2 7 2.6-4.3 6.7 9.8Z" fill="{c}"/><path d="m7.4 12.2 1.6-2.6 1.6 2.6" fill="none" stroke="#fff" stroke-width="1.3" stroke-linejoin="round"/>"##,
        ),
    },
    Os {
        id: "fedora",
        name: "Fedora",
        color: "#51A2DA",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12" r="10" fill="{c}"/><path d="M15.5 6.8h-1.8a2.4 2.4 0 0 0-2.4 2.4V19M8 12.2h6.6" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "centos",
        name: "CentOS",
        color: "#932279",
        glyph: Glyph::Mark(
            r##"<path d="M12 1.5 15 9l7.5 3-7.5 3-3 7.5L9 15l-7.5-3L9 9Z" fill="{c}"/><circle cx="12" cy="12" r="2" fill="#fff"/>"##,
        ),
    },
    Os {
        id: "rocky",
        name: "Rocky Linux",
        color: "#10B981",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12" r="10" fill="{c}"/><path d="m4.5 16.5 5.5-5.5 3.2 3.2L19.5 8" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>"##,
        ),
    },
    Os {
        id: "almalinux",
        name: "AlmaLinux",
        color: "#0F4266",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="6" r="3.4" fill="{c}"/><circle cx="6" cy="16" r="3.4" fill="{c}"/><circle cx="18" cy="16" r="3.4" fill="{c}"/><path d="M12 9.4v3.2M9 14.3l1.6-1M15 14.3l-1.6-1" stroke="{c}" stroke-width="2" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "rhel",
        name: "Red Hat Enterprise Linux",
        color: "#EE0000",
        glyph: Glyph::Mark(
            r##"<path d="M7 12.5 8.8 6c.4-1.3 1.3-2 2.6-2 .9 0 1.6.5 2.4.5S15.4 4 16 4c1.2 0 2 .8 2.3 2l1.4 6.5Z" fill="{c}"/><path d="M2 13.5c0-1 1.8-1.4 3.2-1.4 2.6 1.5 6.6 2.7 11.7 2.7 1.6 0 2.6-.4 2.6-1.4 3 .7 2.9 2 2.4 3-1.1 1.9-5.3 2.6-9.6 2-5.1-.8-10.3-3-10.3-4.9Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "opensuse",
        name: "openSUSE",
        color: "#73BA25",
        glyph: Glyph::Mark(
            r##"<path d="M2 14c0-5 4.5-8.5 10.5-8.5 4.4 0 7.4 1.6 9.5 4.5-1.6-.8-3.4-1-4.6-.4 1.4 1.4 1.4 3.6-.4 4.6 1.9.8 2.6 2.6 1.6 4.3-4-3.3-9.2-.4-12.8.1C3.5 19 2 17 2 14Z" fill="{c}"/><circle cx="16.2" cy="11.4" r="1.4" fill="#fff"/>"##,
        ),
    },
    Os {
        id: "suse",
        name: "SUSE Linux Enterprise",
        color: "#30BA78",
        glyph: Glyph::Mark(
            r##"<path d="M2 14c0-5 4.5-8.5 10.5-8.5 4.4 0 7.4 1.6 9.5 4.5-1.6-.8-3.4-1-4.6-.4 1.4 1.4 1.4 3.6-.4 4.6 1.9.8 2.6 2.6 1.6 4.3-4-3.3-9.2-.4-12.8.1C3.5 19 2 17 2 14Z" fill="{c}"/><circle cx="16.2" cy="11.4" r="1.4" fill="#fff"/>"##,
        ),
    },
    Os {
        id: "nixos",
        name: "NixOS",
        color: "#5277C3",
        glyph: Glyph::Mark(
            r##"<g fill="none" stroke="{c}" stroke-width="2.6" stroke-linecap="round"><path d="M7 3.5 17.5 21.5"/><path d="M17 3.5 6.5 21.5"/><path d="M2 12h20"/></g>"##,
        ),
    },
    Os {
        id: "gentoo",
        name: "Gentoo",
        color: "#54487A",
        glyph: Glyph::Mark(
            r##"<path fill-rule="evenodd" d="M9.5 2.5c5.8-.6 12.4 3.4 12 8-.3 3.3-6.8 6.7-12.6 10.6-2.4 1.6-5 .6-4-1.6.9-2 4.4-4.6 5.3-6.6-3 .2-7.3-1-7.6-3.4C2.2 6.4 5.3 3 9.5 2.5Zm3.4 3.7c-1.5 0-2.4.8-2.4 1.7s1 1.6 2.4 1.6 2.6-.7 2.6-1.6-1.1-1.7-2.6-1.7Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "kali",
        name: "Kali Linux",
        color: "#557C94",
        glyph: Glyph::Mark(
            r##"<path d="M2.5 6.5c4.2-1.6 9.2-1.4 12.6 1.1 2.1 1.6 3.4 4 3.6 6.8.1 1.8 1.4 3.2 3 3.6M9.3 9.6c2.5.2 4.7 1.4 5.8 3.5" fill="none" stroke="{c}" stroke-width="2.2" stroke-linecap="round"/><path d="M14.5 15.5c-.9 2.3-.4 4.4 1.2 6" fill="none" stroke="{c}" stroke-width="1.8" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "linuxmint",
        name: "Linux Mint",
        color: "#87CF3E",
        glyph: Glyph::Mark(
            r##"<path d="M3 4h4v10a3 3 0 0 0 3 3h7a3 3 0 0 0 3-3V8.5a2.5 2.5 0 0 0-5 0V14m0-5.5a2.5 2.5 0 0 0-5 0V14" fill="none" stroke="{c}" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>"##,
        ),
    },
    Os {
        id: "pop",
        name: "Pop!_OS",
        color: "#48B9C7",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12" r="10" fill="{c}"/><path d="M9 17.5V7h3.4a3 3 0 0 1 0 6H9" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/><circle cx="16.5" cy="17" r="1.3" fill="#fff"/>"##,
        ),
    },
    Os {
        id: "elementary",
        name: "elementary OS",
        color: "#64BAFF",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12" r="9" fill="none" stroke="{c}" stroke-width="2"/><path d="M5 15.5c4.5 0 10.5-2 10.5-6.5 0-2-1.4-3-3-3-3.8 0-5 6.6-1 9.5 2.4 1.7 6 1.2 8.5-1" fill="none" stroke="{c}" stroke-width="1.8" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "void",
        name: "Void Linux",
        color: "#478061",
        glyph: Glyph::Mark(
            r##"<path d="M6 6.4A8.5 8.5 0 0 1 20 9.2M18 17.6A8.5 8.5 0 0 1 4 14.8" fill="none" stroke="{c}" stroke-width="2.6" stroke-linecap="round"/><circle cx="12" cy="12" r="3.2" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "raspbian",
        name: "Raspberry Pi OS",
        color: "#A22846",
        glyph: Glyph::Mark(
            r##"<g fill="{c}"><circle cx="12" cy="9.5" r="2.6"/><circle cx="8" cy="13" r="2.6"/><circle cx="16" cy="13" r="2.6"/><circle cx="12" cy="16.6" r="2.6"/><circle cx="9" cy="19.2" r="2"/><circle cx="15" cy="19.2" r="2"/><path d="M12 6.5C10 2.8 6.8 2.6 5.5 3.4 6 5.8 8.6 7.4 11 7.2Zm0 0c2-3.7 5.2-3.9 6.5-3.1-.5 2.4-3.1 4-5.5 3.8Z"/></g>"##,
        ),
    },
    Os {
        id: "ol",
        name: "Oracle Linux",
        color: "#F80000",
        glyph: Glyph::Mark(
            r##"<rect x="2" y="6.5" width="20" height="11" rx="5.5" fill="none" stroke="{c}" stroke-width="2.6"/>"##,
        ),
    },
    Os {
        id: "amzn",
        name: "Amazon Linux",
        color: "#FF9900",
        glyph: Glyph::Mark(
            r##"<path d="M3 15.5c5.4 3.4 12.6 3.4 18 0" fill="none" stroke="{c}" stroke-width="2.2" stroke-linecap="round"/><path d="m17.8 13.6 3.4 1.7-1.4 3.5" fill="none" stroke="{c}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 4h8l3 8H5Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "slackware",
        name: "Slackware",
        color: "#6E7EB4",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12" r="10" fill="{c}"/><path d="M15.6 8.2C14.8 7.3 13.6 7 12.3 7 10.4 7 9 8 9 9.5c0 3.3 6.4 1.9 6.4 5 0 1.6-1.5 2.5-3.5 2.5-1.5 0-2.8-.5-3.6-1.4" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "linux",
        name: "Linux",
        color: "#FCC624",
        glyph: Glyph::Mark(
            r##"<path d="M12 2c-2.6 0-4 2-4 4.8v2.6c0 1-.5 2-1.4 3.2C5 14.6 4.5 17 5.4 19c.6 1.4 2.6 2.4 4 2.4h5.2c1.4 0 3.4-1 4-2.4.9-2 .4-4.4-1.2-6.4-.9-1.2-1.4-2.2-1.4-3.2V6.8C16 4 14.6 2 12 2Z" fill="#2B2B2B" stroke="{c}" stroke-width="1.2"/><ellipse cx="12" cy="15.6" rx="3.6" ry="4.6" fill="#fff"/><path d="M10.4 9.4h3.2L12 11Z" fill="{c}"/><path d="M4.5 19.8c.8-1.8 2.4-2.6 3.8-1.6.8.6.6 1.8-.2 2.6-1 1-2.8 1.2-3.6-1Zm15 0c-.8-1.8-2.4-2.6-3.8-1.6-.8.6-.6 1.8.2 2.6 1 1 2.8 1.2 3.6-1Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "freebsd",
        name: "FreeBSD",
        color: "#AB2B28",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="13" r="8.5" fill="{c}"/><path d="M4.4 7.8C3.2 5.4 2.6 3.6 3.2 3c.7-.6 2.5 0 4.8 1.3M19.6 7.8c1.2-2.4 1.8-4.2 1.2-4.8-.7-.6-2.5 0-4.8 1.3" fill="none" stroke="{c}" stroke-width="1.8" stroke-linecap="round"/>"##,
        ),
    },
    Os {
        id: "openbsd",
        name: "OpenBSD",
        color: "#F2CA30",
        glyph: Glyph::Mark(
            r##"<circle cx="12" cy="12.5" r="7.5" fill="{c}"/><g stroke="{c}" stroke-width="1.6" stroke-linecap="round"><path d="M12 2.5v2M4 6l1.5 1.4M20 6l-1.5 1.4M1.8 12.5h2M20.2 12.5h2M4 19l1.5-1.4M20 19l-1.5-1.4"/></g><circle cx="9.6" cy="11" r="1.2" fill="#1F1F1F"/><circle cx="14.4" cy="11" r="1.2" fill="#1F1F1F"/>"##,
        ),
    },
    Os {
        id: "netbsd",
        name: "NetBSD",
        color: "#FF6600",
        glyph: Glyph::Mark(
            r##"<path d="M5 2.5 6.5 22" stroke="{c}" stroke-width="1.8" stroke-linecap="round"/><path d="M5.6 4.5c4.6-1.6 8 2 15.4.4-1.6 3.4-1.4 6.4-.6 9.4-6.2 1.6-9.6-1.8-13.6-.6Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "proxmox",
        name: "Proxmox VE",
        color: "#E57000",
        glyph: Mark(
            r##"<path d="M1.5 2.5h5L12 8.6l5.5-6.1h5l-8 9.5 8 9.5h-5L12 15.4l-5.5 6.1h-5l8-9.5Z" fill="{c}"/><path d="m8.5 2.5 3.5 4 3.5-4M8.5 21.5l3.5-4 3.5 4" fill="none" stroke="{c}" stroke-opacity=".55" stroke-width="1.2"/>"##,
        ),
    },
    Os {
        id: "truenas",
        name: "TrueNAS",
        color: "#0095D5",
        glyph: Badge(Hexagon, "TN"),
    },
    Os {
        id: "openmediavault",
        name: "OpenMediaVault",
        color: "#5DACDF",
        glyph: Badge(Square, "OMV"),
    },
    Os {
        id: "unraid",
        name: "Unraid",
        color: "#F15A2C",
        glyph: Badge(Square, "U"),
    },
    Os {
        id: "synology",
        name: "Synology DSM",
        color: "#B5B5B6",
        glyph: Badge(Square, "DS"),
    },
    Os {
        id: "qnap",
        name: "QNAP QTS",
        color: "#1D63B5",
        glyph: Badge(Square, "Q"),
    },
    Os {
        id: "pfsense",
        name: "pfSense",
        color: "#1475CF",
        glyph: Badge(Shield, "PF"),
    },
    Os {
        id: "opnsense",
        name: "OPNsense",
        color: "#D94F00",
        glyph: Badge(Shield, "OPN"),
    },
    Os {
        id: "vyos",
        name: "VyOS",
        color: "#F09A1B",
        glyph: Badge(Hexagon, "VY"),
    },
    Os {
        id: "openwrt",
        name: "OpenWrt",
        color: "#00B5E2",
        glyph: Mark(
            r##"<g fill="none" stroke="{c}" stroke-width="2.2" stroke-linecap="round"><path d="M2.5 9.5a13.5 13.5 0 0 1 19 0"/><path d="M5.8 13a8.8 8.8 0 0 1 12.4 0"/><path d="M9 16.5a4.3 4.3 0 0 1 6 0"/></g><circle cx="12" cy="20" r="1.8" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "haos",
        name: "Home Assistant OS",
        color: "#18BCF2",
        glyph: Mark(
            r##"<path d="M12 2 1.5 11.5h3V22h15V11.5h3Z" fill="{c}"/><g fill="none" stroke="#fff" stroke-width="1.4" stroke-linecap="round"><path d="M12 20.5v-9M12 15l-3-3M12 17l3-3"/></g><g fill="#fff"><circle cx="12" cy="11" r="1.3"/><circle cx="8.6" cy="11.6" r="1.3"/><circle cx="15.4" cy="13.6" r="1.3"/></g>"##,
        ),
    },
    Os {
        id: "docker",
        name: "Docker",
        color: "#2496ED",
        glyph: Mark(
            r##"<path d="M2 12h17.5c.9-1.2 1.4-2.6 1.4-3.5 1 .2 1.8.8 2.1 1.6-.6.6-1.6.8-2.6.6C18.8 16.5 15 20 9 20c-4 0-6.4-3.3-7-8Z" fill="{c}"/><g fill="{c}"><rect x="4" y="8.5" width="2.6" height="2.6"/><rect x="7.2" y="8.5" width="2.6" height="2.6"/><rect x="10.4" y="8.5" width="2.6" height="2.6"/><rect x="13.6" y="8.5" width="2.6" height="2.6"/><rect x="7.2" y="5.3" width="2.6" height="2.6"/><rect x="10.4" y="5.3" width="2.6" height="2.6"/><rect x="10.4" y="2.1" width="2.6" height="2.6"/></g>"##,
        ),
    },
    Os {
        id: "kubernetes",
        name: "Kubernetes",
        color: "#326CE5",
        glyph: Mark(
            r##"<path d="M12 1.5 21.4 6l2.3 10.2-6.5 8.1H6.8L.3 16.2 2.6 6Z" fill="{c}" transform="matrix(.92 0 0 .92 .96 .5)"/><circle cx="12" cy="12" r="3.6" fill="none" stroke="#fff" stroke-width="1.5"/><g stroke="#fff" stroke-width="1.5" stroke-linecap="round"><path d="M12 4.6v3.8M12 15.6v3.8M5 9.7l3.6 1.2M15.4 13.1l3.6 1.2M5 14.3l3.6-1.2M15.4 10.9 19 9.7M8 18l2.1-3M13.9 9 16 6M8 6l2.1 3M13.9 15l2.1 3"/></g>"##,
        ),
    },
    Os {
        id: "endeavouros",
        name: "EndeavourOS",
        color: "#7F3FBF",
        glyph: Badge(Circle, "E"),
    },
    Os {
        id: "garuda",
        name: "Garuda Linux",
        color: "#3A86FF",
        glyph: Badge(Circle, "G"),
    },
    Os {
        id: "artix",
        name: "Artix Linux",
        color: "#10A0CC",
        glyph: Badge(Hexagon, "AX"),
    },
    Os {
        id: "cachyos",
        name: "CachyOS",
        color: "#00CCFF",
        glyph: Badge(Circle, "C"),
    },
    Os {
        id: "zorin",
        name: "Zorin OS",
        color: "#15A6F0",
        glyph: Badge(Square, "Z"),
    },
    Os {
        id: "neon",
        name: "KDE neon",
        color: "#20A6A4",
        glyph: Badge(Circle, "KN"),
    },
    Os {
        id: "kubuntu",
        name: "Kubuntu",
        color: "#0079C1",
        glyph: Badge(Circle, "K"),
    },
    Os {
        id: "deepin",
        name: "deepin",
        color: "#007CFF",
        glyph: Badge(Circle, "D"),
    },
    Os {
        id: "devuan",
        name: "Devuan",
        color: "#4C4A6E",
        glyph: Badge(Circle, "DV"),
    },
    Os {
        id: "mx",
        name: "MX Linux",
        color: "#2E2E2E",
        glyph: Badge(Square, "MX"),
    },
    Os {
        id: "pureos",
        name: "PureOS",
        color: "#2FB6E5",
        glyph: Badge(Circle, "P"),
    },
    Os {
        id: "parrot",
        name: "Parrot OS",
        color: "#05C5D9",
        glyph: Badge(Shield, "PR"),
    },
    Os {
        id: "tails",
        name: "Tails",
        color: "#56347C",
        glyph: Badge(Shield, "T"),
    },
    Os {
        id: "solus",
        name: "Solus",
        color: "#5294E2",
        glyph: Badge(Circle, "S"),
    },
    Os {
        id: "mageia",
        name: "Mageia",
        color: "#2397D4",
        glyph: Badge(Circle, "MG"),
    },
    Os {
        id: "pclinuxos",
        name: "PCLinuxOS",
        color: "#2D6FB7",
        glyph: Badge(Square, "PC"),
    },
    Os {
        id: "nobara",
        name: "Nobara",
        color: "#8A4FFF",
        glyph: Badge(Circle, "N"),
    },
    Os {
        id: "bazzite",
        name: "Bazzite",
        color: "#8C66FF",
        glyph: Badge(Circle, "BZ"),
    },
    Os {
        id: "ultramarine",
        name: "Ultramarine Linux",
        color: "#1A73E8",
        glyph: Badge(Circle, "UM"),
    },
    Os {
        id: "fedora-coreos",
        name: "Fedora CoreOS",
        color: "#3C6EB4",
        glyph: Badge(Hexagon, "FC"),
    },
    Os {
        id: "flatcar",
        name: "Flatcar",
        color: "#09BAC8",
        glyph: Badge(Hexagon, "FL"),
    },
    Os {
        id: "talos",
        name: "Talos Linux",
        color: "#FF7300",
        glyph: Badge(Hexagon, "T"),
    },
    Os {
        id: "bottlerocket",
        name: "Bottlerocket",
        color: "#FF9900",
        glyph: Badge(Hexagon, "BR"),
    },
    Os {
        id: "photon",
        name: "Photon OS",
        color: "#1D73B9",
        glyph: Badge(Hexagon, "PH"),
    },
    Os {
        id: "azurelinux",
        name: "Azure Linux",
        color: "#0078D4",
        glyph: Badge(Hexagon, "AZ"),
    },
    Os {
        id: "cos",
        name: "Container-Optimized OS",
        color: "#4285F4",
        glyph: Badge(Hexagon, "COS"),
    },
    Os {
        id: "wolfi",
        name: "Wolfi",
        color: "#4445E7",
        glyph: Badge(Hexagon, "W"),
    },
    Os {
        id: "clear-linux-os",
        name: "Clear Linux",
        color: "#0071C5",
        glyph: Badge(Square, "CL"),
    },
    Os {
        id: "cloudlinux",
        name: "CloudLinux",
        color: "#0097F3",
        glyph: Badge(Square, "CLN"),
    },
    Os {
        id: "eurolinux",
        name: "EuroLinux",
        color: "#00559D",
        glyph: Badge(Square, "EL"),
    },
    Os {
        id: "scientific",
        name: "Scientific Linux",
        color: "#2F5BA6",
        glyph: Badge(Square, "SL"),
    },
    Os {
        id: "openeuler",
        name: "openEuler",
        color: "#002FA7",
        glyph: Badge(Square, "OE"),
    },
    Os {
        id: "anolis",
        name: "Anolis OS",
        color: "#FF6A00",
        glyph: Badge(Square, "AN"),
    },
    Os {
        id: "kylin",
        name: "Kylin",
        color: "#EA4033",
        glyph: Badge(Square, "KY"),
    },
    Os {
        id: "uos",
        name: "UnionTech OS",
        color: "#0081FF",
        glyph: Badge(Square, "UOS"),
    },
    Os {
        id: "astra",
        name: "Astra Linux",
        color: "#1D5CA6",
        glyph: Badge(Shield, "AS"),
    },
    Os {
        id: "altlinux",
        name: "ALT Linux",
        color: "#F5B400",
        glyph: Badge(Circle, "ALT"),
    },
    Os {
        id: "redos",
        name: "RED OS",
        color: "#CD2026",
        glyph: Badge(Circle, "RED"),
    },
    Os {
        id: "rosa",
        name: "ROSA Linux",
        color: "#2C9FD9",
        glyph: Badge(Circle, "R"),
    },
    Os {
        id: "guix",
        name: "Guix System",
        color: "#FFCC00",
        glyph: Badge(Circle, "GX"),
    },
    Os {
        id: "chimera",
        name: "Chimera Linux",
        color: "#E03D50",
        glyph: Badge(Circle, "CH"),
    },
    Os {
        id: "postmarketos",
        name: "postmarketOS",
        color: "#009900",
        glyph: Badge(Circle, "PM"),
    },
    Os {
        id: "steamos",
        name: "SteamOS",
        color: "#1A9FFF",
        glyph: Badge(Circle, "SO"),
    },
    Os {
        id: "chromeos",
        name: "ChromeOS",
        color: "#4285F4",
        glyph: Badge(Circle, "CR"),
    },
    Os {
        id: "dragonfly",
        name: "DragonFly BSD",
        color: "#6A3E93",
        glyph: Badge(Shield, "DF"),
    },
    Os {
        id: "omnios",
        name: "OmniOS",
        color: "#0A87C9",
        glyph: Badge(Shield, "OO"),
    },
    Os {
        id: "openindiana",
        name: "OpenIndiana",
        color: "#6DB0D9",
        glyph: Badge(Shield, "OI"),
    },
    Os {
        id: "solaris",
        name: "Oracle Solaris",
        color: "#C74634",
        glyph: Badge(Shield, "SOL"),
    },
    Os {
        id: "macos",
        name: "macOS",
        color: "#A2AAAD",
        glyph: Glyph::Mark(
            r##"<path d="M16.4 12.7c0-2.3 1.9-3.4 2-3.5-1.1-1.6-2.8-1.8-3.4-1.8-1.4-.1-2.8.9-3.5.9-.7 0-1.9-.8-3-.8-1.6 0-3 .9-3.8 2.3-1.6 2.8-.4 7 1.2 9.2.8 1.1 1.7 2.4 2.9 2.3 1.1 0 1.6-.7 3-.7s1.8.7 3 .7c1.3 0 2-1.1 2.8-2.2.9-1.3 1.2-2.5 1.3-2.6-.1 0-2.5-.9-2.5-3.8ZM14.1 5.9c.6-.8 1.1-1.8 1-2.9-.9 0-2.1.6-2.7 1.4-.6.7-1.1 1.8-1 2.8 1 .1 2.1-.5 2.7-1.3Z" fill="{c}"/>"##,
        ),
    },
    Os {
        id: "windows",
        name: "Windows",
        color: "#0078D4",
        glyph: Glyph::Mark(
            r##"<path d="M2.5 4.5 10.5 3.4v7.9h-8Zm9.2-1.3L21.5 2v9.3h-9.8Zm-9.2 9.3h8v7.9l-8-1.1Zm9.2 0h9.8V22l-9.8-1.3Z" fill="{c}"/>"##,
        ),
    },
];
