//! SSH-suite-only assertions/setup over the shared opaque real server.
use super::*;
pub(super) trait SshdKeys {
    fn host_label(&self) -> String;
    fn host_fingerprint(&self) -> String;
}
impl SshdKeys for Sshd {
    fn host_label(&self) -> String {
        format!("[127.0.0.1]:{}", self.port())
    }
    fn host_fingerprint(&self) -> String {
        let output = Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(self.root().join("host_key.pub"))
            .output()
            .unwrap();
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned()
    }
}
