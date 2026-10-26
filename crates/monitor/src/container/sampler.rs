use super::{ContainerCounters, ContainerSnapshot};
use crate::Usage;

#[derive(Clone, Debug, Default)]
pub(crate) struct ContainerSampler {
    previous: Option<(f64, u64, String)>,
}
pub(crate) struct Sample {
    pub(crate) cpu: Option<f32>,
    pub(crate) memory: Option<Usage>,
    pub(crate) facts: ContainerSnapshot,
}
impl ContainerSampler {
    pub(crate) fn sample(
        &mut self,
        clock: Option<f64>,
        counters: Option<ContainerCounters>,
    ) -> Option<Sample> {
        let previous = self.previous.take();
        let counters = counters?;
        let cpu = clock
            .zip(counters.cpu_nanoseconds)
            .zip(counters.cpu_group)
            .and_then(|((clock, usage), group)| {
                self.previous = Some((clock, usage, group.clone()));
                let (then, before, previous_group) = previous?;
                if previous_group != group {
                    return None;
                }
                let elapsed = clock - then;
                let delta = usage.checked_sub(before)?;
                if elapsed <= 0. || !elapsed.is_finite() {
                    return None;
                }
                let percent = delta as f64 / 1_000_000_000. / elapsed * 100.;
                (percent.is_finite() && percent <= f64::from(f32::MAX)).then_some(percent as f32)
            });
        Some(Sample {
            cpu,
            memory: counters
                .memory_current
                .zip(counters.memory_limit)
                .map(|(used, total)| Usage { used, total }),
            facts: ContainerSnapshot {
                memory_current: counters.memory_current,
                memory_limit: counters.memory_limit,
            },
        })
    }
}
