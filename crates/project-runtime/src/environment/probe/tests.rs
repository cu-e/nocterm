use super::*;
fn fixture(id: &str, libc: &str) -> String {
    format!(
        "kernel\x00Linux\x00architecture\x00x86_64\x00os_release\x00ID={id}\nPRETTY_NAME=\"Observed OS\"\nVERSION_ID=\"1\"\x00libc\x00{libc}\x00uid\x001000\x00user\x00dev\x00home\x00/home/dev\x00writable\x00yes\x00program\x00curl\x00python\x00Python 3.13.2\x00rust\x00\x00bun\x00\x00node\x00v22.1.0\x00java\x00\x00"
    )
}
#[test]
fn real_release_families_and_versions_are_observed_as_data() {
    for (id, family) in [
        ("ubuntu", DistributionFamily::Debian),
        ("debian", DistributionFamily::Debian),
        ("kali", DistributionFamily::Debian),
        ("alpine", DistributionFamily::Alpine),
        ("arch", DistributionFamily::Arch),
    ] {
        let observed = decode(&fixture(id, "musl libc")).unwrap();
        assert_eq!(observed.os.family, family);
        assert_eq!(observed.account.home, "/home/dev");
        assert_eq!(observed.tools[0].version.as_deref(), Some("Python 3.13.2"));
        assert_eq!(observed.os.libc, LibcFamily::Musl);
    }
}
#[test]
fn aliases_and_hostile_os_release_are_never_executed() {
    let mut text = fixture("unknown", "glibc 2.39");
    text = text.replace(
        "ID=unknown",
        "ID=unknown\nID_LIKE=\"arch\"\nEVIL=$(touch /host)",
    );
    let observed = decode(&text).unwrap();
    assert_eq!(observed.os.family, DistributionFamily::Arch);
    assert_eq!(observed.os.libc, LibcFamily::Glibc);
    assert_eq!(observed.os.name, "Observed OS");
}
#[test]
fn unsafe_home_and_truncated_probe_output_are_typed_refusals() {
    assert!(decode(&fixture("debian", "glibc").replace("/home/dev", "relative")).is_err());
    assert!(decode("kernel\x00").is_err());
}
