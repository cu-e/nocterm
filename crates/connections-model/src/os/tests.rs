use super::*;
#[test]
fn os_release_id_wins_then_id_like_then_generic_linux() {
    let ubuntu = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";
    assert_eq!(from_os_release(ubuntu).unwrap().as_str(), "ubuntu");
    let derived = "ID=\"bodhi\"\nID_LIKE=\"ubuntu debian\"\n";
    assert_eq!(from_os_release(derived).unwrap().as_str(), "ubuntu");
    let leap = "ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"";
    assert_eq!(from_os_release(leap).unwrap().as_str(), "opensuse");
    let alma = "ID=\"almalinux\"\nID_LIKE=\"rhel centos fedora\"";
    assert_eq!(from_os_release(alma).unwrap().as_str(), "almalinux");
    let coreos = "ID=fedora\nVARIANT_ID=coreos\n";
    assert_eq!(from_os_release(coreos).unwrap().as_str(), "fedora-coreos");
    let alt = "ID=altlinux\n";
    assert_eq!(from_os_release(alt).unwrap().as_str(), "altlinux");
    let unknown = "ID=exotic\n";
    assert_eq!(from_os_release(unknown).unwrap().as_str(), "linux");
    assert_eq!(from_os_release("# no id\nNAME=x"), None);
    assert_eq!(from_os_release("  ID = 'Arch'  ").unwrap().as_str(), "arch");
}

#[test]
fn aliases_preserve_canonical_identity() {
    for (alias, id) in [
        ("archarm", "arch"),
        ("manjaro-arm", "manjaro"),
        ("redhat", "rhel"),
        ("centos-stream", "centos"),
        ("raspberrypi", "raspbian"),
        ("mariner", "azurelinux"),
        ("alt", "altlinux"),
        ("hassos", "haos"),
        ("chromiumos", "chromeos"),
    ] {
        assert_eq!(
            from_os_release(&format!("ID={alias}")).unwrap().as_str(),
            id
        );
    }
}
#[test]
fn windows_home_drive_shapes() {
    for home in ["/C:/Users/a", "C:/Users/a", "z:\\Users"] {
        assert!(is_windows_home(home));
    }
    for home in ["/root", "", "1:/home", "/:x"] {
        assert!(!is_windows_home(home));
    }
}

#[test]
fn appliance_marker_eligibility_and_priority_are_pure() {
    let debian: Vec<_> = platform_markers(OsId::new("debian"))
        .map(|(_, id)| id.as_str())
        .collect();
    assert_eq!(debian, ["proxmox", "truenas", "openmediavault", "vyos"]);
    assert!(platform_markers(OsId::new("ubuntu")).next().is_none());
    let without_release: Vec<_> = platform_markers(None).map(|(_, id)| id.as_str()).collect();
    assert_eq!(
        without_release,
        [
            "truenas", "unraid", "opnsense", "pfsense", "synology", "qnap"
        ]
    );
}
