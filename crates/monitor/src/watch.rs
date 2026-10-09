//! One running collection script and the readings it prints.

use std::{collections::VecDeque, time::Duration};

use nocterm_session::{ExecError, ExecOutput, HostExec};

use crate::{MetricSet, Platform, Reading, frame::FrameReader};

/// A collection script running on a host. Dropping it stops the script.
pub struct Watch {
    platform: Platform,
    output: ExecOutput,
    frames: FrameReader,
    pending: VecDeque<Vec<String>>,
}

impl Watch {
    /// Starts watching `metrics`, a frame every `interval`.
    pub async fn start(
        exec: &dyn HostExec,
        platform: Platform,
        metrics: MetricSet,
        interval: Duration,
    ) -> Result<Self, ExecError> {
        let request = platform
            .watch_request(metrics, interval)
            .ok_or(ExecError::Unsupported)?;
        let output = exec.exec(request).await?;
        Ok(Self {
            platform,
            output,
            frames: FrameReader::default(),
            pending: VecDeque::new(),
        })
    }

    /// The next reading, or `None` once the script has ended.
    pub async fn next(&mut self) -> Option<Reading> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                return Some(self.platform.parse(&frame));
            }
            let chunk = self.output.next().await?;
            self.pending.extend(self.frames.push(&chunk));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::MonitorMetric;

    use super::*;
    use crate::platform::tests::Scripted;

    #[test]
    fn readings_follow_the_frames() {
        let exec = Scripted {
            answers: vec![("sh", Ok("@@uptime\n10.0\n@@end\n@@uptime\n11.0\n@@end\n"))],
            asked: Arc::new(Mutex::default()),
        };
        futures::executor::block_on(async {
            let metrics = [MonitorMetric::Uptime].iter().collect();
            let mut watch = Watch::start(&exec, Platform::Linux, metrics, Duration::from_secs(5))
                .await
                .unwrap();
            assert_eq!(watch.next().await.unwrap().uptime, Some(10.0));
            assert_eq!(watch.next().await.unwrap().uptime, Some(11.0));
            assert_eq!(watch.next().await, None);
        });
        let unsupported = futures::executor::block_on(Watch::start(
            &exec,
            Platform::Unsupported("Darwin".into()),
            MetricSet::all(),
            Duration::from_secs(5),
        ));
        assert!(matches!(unsupported, Err(ExecError::Unsupported)));
    }
}
