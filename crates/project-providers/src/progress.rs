use crate::SetupPhase;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferProgress {
    pub subject: Option<String>,
    pub current_bytes: u64,
    pub total_bytes: Option<u64>,
}
impl TransferProgress {
    pub fn new(subject: Option<&str>, current_bytes: u64, total_bytes: Option<u64>) -> Self {
        let subject = subject.map(|value| {
            value
                .chars()
                .filter(|ch| !ch.is_control())
                .take(128)
                .collect()
        });
        Self {
            subject,
            current_bytes,
            total_bytes: total_bytes.filter(|total| *total >= current_bytes && *total > 0),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupUpdate {
    pub phase: SetupPhase,
    pub transfer: Option<TransferProgress>,
}
impl SetupUpdate {
    pub fn phase(phase: SetupPhase) -> Self {
        Self {
            phase,
            transfer: None,
        }
    }
    pub fn label(&self) -> &'static str {
        self.phase.label()
    }
}
impl From<SetupPhase> for SetupUpdate {
    fn from(phase: SetupPhase) -> Self {
        Self::phase(phase)
    }
}
