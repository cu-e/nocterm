//! Own-cgroup CPU and memory facts; no parent-host counter substitution.
mod membership;
mod sampler;
pub(crate) use sampler::ContainerSampler;
mod script;
use crate::{MetricSet, Reading};
use nocterm_session::ExecRequest;
use std::time::Duration;

/// Cumulative CPU nanoseconds and cgroup memory bytes, independent of host totals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerCounters {
    pub cpu_nanoseconds: Option<u64>,
    pub cpu_group: Option<String>,
    pub memory_current: Option<u64>,
    /// None means unlimited or unavailable, never host memory capacity.
    pub memory_limit: Option<u64>,
}
/// Current own-cgroup memory and an explicit one-CPU utilization basis.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerSnapshot {
    pub memory_current: Option<u64>,
    pub memory_limit: Option<u64>,
}
pub(crate) fn request(metrics: MetricSet, interval: Duration) -> ExecRequest {
    script::request(metrics, interval)
}
pub(crate) fn parse(frame: &[String]) -> Reading {
    let mut counters = ContainerCounters::default();
    let mut clock = None;
    let members = section(frame, "@@cgroup");
    let mounts = section(frame, "@@mountinfo");
    let selected = section(frame, "@@selected");
    let cpu_version = value(&selected, "cpu_version");
    let memory_version = value(&selected, "memory_version");
    let cpu = verified(
        &members,
        &mounts,
        &selected,
        "cpu_path",
        if cpu_version == Some("2") {
            "v2"
        } else {
            "cpuacct"
        },
    );
    let memory = verified(
        &members,
        &mounts,
        &selected,
        "memory_path",
        if memory_version == Some("2") {
            "v2"
        } else {
            "memory"
        },
    );
    for line in section(frame, "@@container") {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "clock" => {
                clock = value
                    .parse::<f64>()
                    .ok()
                    .filter(|clock| clock.is_finite() && *clock >= 0.)
            }
            "cpu_us" if cpu.is_some() && cpu_version == Some("2") => {
                counters.cpu_nanoseconds = value
                    .parse::<u64>()
                    .ok()
                    .and_then(|value| value.checked_mul(1000))
            }
            "cpu_ns" if cpu.is_some() && cpu_version == Some("1") => {
                counters.cpu_nanoseconds = value.parse().ok()
            }
            "memory" if memory.is_some() => counters.memory_current = value.parse().ok(),
            "limit" if memory.is_some() && memory_version == Some("2") => {
                counters.memory_limit = value.parse().ok().filter(|value| *value > 0)
            }
            "limit_v1" if memory.is_some() && memory_version == Some("1") => {
                counters.memory_limit = value
                    .parse()
                    .ok()
                    .filter(|value| *value > 0 && *value < (1u64 << 60) && *value != 0x7ffff000)
            }
            _ => {}
        }
    }
    counters.cpu_group = cpu;
    Reading {
        clock,
        container: Some(counters),
        ..Reading::default()
    }
}
fn section<'a>(frame: &'a [String], name: &str) -> Vec<&'a str> {
    frame
        .iter()
        .skip_while(|line| line.as_str() != name)
        .skip(1)
        .take_while(|line| !line.starts_with("@@"))
        .map(String::as_str)
        .collect()
}
fn value<'a>(section: &[&'a str], key: &str) -> Option<&'a str> {
    let mut values = section
        .iter()
        .filter_map(|line| line.split_once(' '))
        .filter(|(field, _)| *field == key)
        .map(|(_, value)| value);
    let value = values.next()?;
    values.next().is_none().then_some(value)
}
fn verified(
    members: &[&str],
    mounts: &[&str],
    selected: &[&str],
    key: &str,
    controller: &str,
) -> Option<String> {
    let actual = value(selected, key)?;
    let expected = membership::resolve(members, mounts, controller)?;
    (!actual.is_empty() && actual == expected).then_some(format!("{controller}:{expected}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sampler;
    fn frame(version: u8, path: &str, cpu: u64, memory: &str, limit: &str) -> Vec<String> {
        let (member, mount, unit, limit_key) = if version == 2 {
            (
                "0::/a",
                "1 2 0:1 / /cg ro - cgroup2 cgroup rw",
                "cpu_us",
                "limit",
            )
        } else {
            (
                "2:cpuacct,memory:/a",
                "1 2 0:1 / /cg ro - cgroup cgroup rw,cpuacct,memory",
                "cpu_ns",
                "limit_v1",
            )
        };
        format!("@@cgroup\n{member}\n@@mountinfo\n{mount}\n@@selected\ncpu_path {path}\nmemory_path {path}\ncpu_version {version}\nmemory_version {version}\n@@container\nclock 10\n{unit} {cpu}\nmemory {memory}\n{limit_key} {limit}\n").lines().map(str::to_owned).collect()
    }
    #[test]
    fn v2_and_v1_use_their_declared_cpu_units_and_memory_scope() {
        for (version, cpu) in [(2, 123), (1, 123000)] {
            let reading = parse(&frame(version, "/cg/a", cpu, "55", "100"));
            let counters = reading.container.unwrap();
            assert_eq!(counters.cpu_nanoseconds, Some(123000));
            assert_eq!(counters.memory_current, Some(55));
            assert_eq!(counters.memory_limit, Some(100));
            assert!(
                reading.cpu.is_none()
                    && reading.memory.is_none()
                    && reading.cores.is_none()
                    && reading.load.is_none()
                    && reading.uptime.is_none()
            );
        }
    }
    #[test]
    fn limits_and_invalid_membership_never_substitute_parent_host_capacity() {
        for (version, limit) in [(2, "max"), (1, "9223372036854771712"), (1, "2147479552")] {
            let reading = parse(&frame(version, "/cg/a", 10, "55", limit));
            let counters = reading.container.unwrap();
            assert_eq!(counters.memory_limit, None);
            assert_eq!(counters.memory_current, Some(55));
        }
        let reading = parse(&frame(2, "/cg", 10, "55", "100"));
        let counters = reading.container.unwrap();
        assert_eq!(counters.cpu_nanoseconds, None);
        assert_eq!(counters.memory_current, None);
        let mut missing = frame(2, "/cg/a", 10, "55", "100");
        missing.retain(|line| !line.starts_with("1 2"));
        assert_eq!(
            parse(&missing).container.unwrap(),
            ContainerCounters::default()
        );
    }
    fn reading(clock: f64, usage: u64, group: &str, limit: Option<u64>) -> Reading {
        Reading {
            clock: Some(clock),
            container: Some(ContainerCounters {
                cpu_nanoseconds: Some(usage),
                cpu_group: Some(group.into()),
                memory_current: Some(55),
                memory_limit: limit,
            }),
            ..Reading::default()
        }
    }
    #[test]
    fn one_cpu_basis_can_exceed_one_hundred_and_resets_on_clock_counter_or_group_changes() {
        let mut sampler = Sampler::new(8);
        sampler.apply(reading(10., 1_000_000_000, "a", Some(100)));
        sampler.apply(reading(11., 3_000_000_000, "a", Some(100)));
        assert_eq!(sampler.snapshot().cpu, Some(200.));
        assert_eq!(sampler.snapshot().memory.unwrap().percent(), 55.);
        sampler.apply(reading(12., 1, "a", None));
        assert_eq!(sampler.snapshot().cpu, None);
        assert_eq!(sampler.snapshot().memory, None);
        assert_eq!(
            sampler
                .snapshot()
                .container
                .as_ref()
                .unwrap()
                .memory_current,
            Some(55)
        );
        sampler.apply(reading(13., 1_000_000_001, "b", None));
        assert_eq!(sampler.snapshot().cpu, None);
        sampler.apply(reading(1., 2_000_000_001, "b", None));
        assert_eq!(sampler.snapshot().cpu, None);
        sampler.apply(Reading::default());
        assert!(sampler.snapshot().container.is_none());
    }
    #[test]
    fn resource_script_never_reads_host_metric_sources_or_unrequested_counters() {
        let script = script::request(
            [crate::MonitorMetric::Memory].iter().collect(),
            Duration::from_secs(5),
        );
        let script = String::from_utf8(script.stdin.unwrap()).unwrap();
        assert!(!script.contains("/proc/stat"));
        assert!(!script.contains("/proc/meminfo"));
        assert!(!script.contains("/proc/loadavg"));
        assert!(!script.contains("cpu_us %s"));
        assert!(script.contains("memory.current"));
        assert!(script.contains("cgroup.procs"));
        assert!(script.contains("/proc/self/mountinfo"));
    }
}
