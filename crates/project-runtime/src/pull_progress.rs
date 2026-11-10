//! Bounded best-effort engine progress decoding. A layer total is never a global percentage.
use nocterm_project_providers::TransferProgress;
use std::time::{Duration, Instant};
pub(crate) struct PullProgressDecoder {
    pending: Vec<u8>,
    last: Option<TransferProgress>,
    emitted: Option<Instant>,
    ansi: bool,
}
impl PullProgressDecoder {
    pub(crate) fn new() -> Self {
        Self {
            pending: Vec::new(),
            last: None,
            emitted: None,
            ansi: false,
        }
    }
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Option<TransferProgress> {
        let mut observed = None;
        for byte in bytes {
            if self.ansi {
                if byte.is_ascii_alphabetic() {
                    self.ansi = false;
                }
                continue;
            }
            match byte {
                27 => self.ansi = true,
                b'\n' | b'\r' => {
                    if let Some(progress) = parse(&self.pending) {
                        self.last = Some(progress.clone());
                        observed = Some(progress);
                    }
                    self.pending.clear();
                }
                _ => {
                    if self.pending.len() < 4096 {
                        self.pending.push(*byte);
                    }
                }
            }
        }
        if observed.is_some()
            && self
                .emitted
                .is_none_or(|last| last.elapsed() >= Duration::from_millis(100))
        {
            self.emitted = Some(Instant::now());
            observed
        } else {
            None
        }
    }
    pub(crate) fn latest(&mut self) -> Option<TransferProgress> {
        if let Some(progress) = parse(&self.pending) {
            self.last = Some(progress);
        }
        self.last.clone()
    }
}
fn parse(bytes: &[u8]) -> Option<TransferProgress> {
    let line = std::str::from_utf8(bytes).ok()?;
    for token in line.split_whitespace() {
        if let Some((current, total)) = token.split_once('/') {
            let current = quantity(current)?;
            let total = quantity(total)?;
            let subject = line.split(':').next().filter(|_| line.contains(':'));
            return Some(TransferProgress::new(subject, current, Some(total)));
        }
    }
    None
}
fn quantity(value: &str) -> Option<u64> {
    let boundary = value.find(|ch: char| !ch.is_ascii_digit() && ch != '.')?;
    let amount: f64 = value[..boundary].parse().ok()?;
    let factor = match &value[boundary..] {
        "B" => 1.,
        "kB" | "KB" => 1000.,
        "MB" => 1_000_000.,
        "GB" => 1_000_000_000.,
        "KiB" => 1024.,
        "MiB" => 1_048_576.,
        "GiB" => 1_073_741_824.,
        _ => return None,
    };
    let bytes = amount * factor;
    (bytes.is_finite() && bytes >= 0. && bytes < u64::MAX as f64).then_some(bytes as u64)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_cr_and_ansi_report_only_real_layer_bytes() {
        let mut decoder = PullProgressDecoder::new();
        assert!(decoder.push(b"\x1b[2Kabc: Downloading [=>] 1.5M").is_none());
        let progress = decoder.push(b"B/3MB\r").unwrap();
        assert_eq!(progress.current_bytes, 1_500_000);
        assert_eq!(progress.total_bytes, Some(3_000_000));
        assert_eq!(progress.subject.as_deref(), Some("abc"));
    }
    #[test]
    fn unknown_malformed_and_oversized_lines_never_invent_totals() {
        let mut decoder = PullProgressDecoder::new();
        assert!(
            decoder
                .push(b"Already exists\nPull complete\n1EB/2EB\r")
                .is_none()
        );
        assert!(decoder.latest().is_none());
        decoder.push(&vec![b'x'; 8192]);
        assert!(decoder.latest().is_none());
    }
    #[test]
    fn final_latest_value_survives_throttling() {
        let mut decoder = PullProgressDecoder::new();
        decoder.push(b"a: Downloading 1MB/4MB\r").unwrap();
        assert!(decoder.push(b"a: Downloading 3MB/4MB\r").is_none());
        assert_eq!(decoder.latest().unwrap().current_bytes, 3_000_000);
    }
}
