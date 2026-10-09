//! Linux hosts: a POSIX `sh` script reading `/proc` and `/sys`.
//!
//! The script travels on standard input to `sh -s`, so it runs the same
//! under any login shell (bash, zsh, fish, busybox). It sticks to shell
//! builtins: only `sleep`, and `df` when disk space is asked for, fork.

mod parse;

use std::{fmt::Write as _, time::Duration};

use crate::MonitorMetric;
use nocterm_session::ExecRequest;

pub(crate) use parse::parse;

use crate::MetricSet;

/// The watch request for `metrics`, printing a frame every `interval`.
pub(crate) fn request(metrics: MetricSet, interval: Duration) -> ExecRequest {
    ExecRequest::new("sh")
        .arg("-s")
        .stdin(script(metrics, interval))
}

/// Host details, printed by the first frame only.
const HOST: &str = r#"if [ -n "$H" ]; then
printf '@@host\n'
read -r v </proc/sys/kernel/hostname && printf 'hostname %s\n' "$v"
read -r v </proc/sys/kernel/osrelease && printf 'kernel %s\n' "$v"
[ -r /etc/os-release ] && while IFS='=' read -r k v; do [ "$k" = PRETTY_NAME ] && printf 'os %s\n' "$v"; done </etc/os-release
while IFS=: read -r k v; do case $k in 'model name'*|Model*|Hardware*) printf 'cpu %s\n' "$v"; break;; esac; done </proc/cpuinfo
fi
"#;

const STAT: &str = r#"printf '@@stat\n'
while read -r n r; do case $n in cpu*) printf '%s %s\n' "$n" "$r";; *) break;; esac; done </proc/stat
"#;

const MEMINFO: &str = r#"printf '@@meminfo\n'
while read -r k v _; do case $k in MemTotal:|MemFree:|MemAvailable:|Buffers:|Cached:|SwapTotal:|SwapFree:) printf '%s %s\n' "$k" "$v";; esac; done </proc/meminfo
"#;

const LOADAVG: &str = r#"printf '@@loadavg\n'
read -r a b c _ </proc/loadavg && printf '%s %s %s\n' "$a" "$b" "$c"
"#;

const NETDEV: &str = r#"printf '@@netdev\n'
{ read -r _; read -r _; while read -r l; do printf '%s\n' "$l"; done; } </proc/net/dev
"#;

const DISKSTATS: &str = r#"printf '@@blocks\n'
for d in /sys/block/*; do [ -e "$d" ] && printf '%s\n' "${d##*/}"; done
printf '@@diskstats\n'
while read -r l; do printf '%s\n' "$l"; done </proc/diskstats
"#;

// `df -l` keeps network mounts out (they may hang); busybox lacks it.
const DF: &str = r#"printf '@@df\n'
$T df -kPl 2>/dev/null || $T df -kP
"#;

const TEMP: &str = r#"printf '@@temp\n'
s=
for h in /sys/class/hwmon/hwmon*; do
[ -r "$h/name" ] || continue
read -r n <"$h/name"
for t in "$h"/temp*_input; do
[ -r "$t" ] && read -r v <"$t" || continue
l=; [ -r "${t%_input}_label" ] && read -r l <"${t%_input}_label"
printf '%s %s %s\n' "$v" "$n" "$l"; s=1
done
done
if [ -z "$s" ]; then
for z in /sys/class/thermal/thermal_zone*; do
[ -r "$z/temp" ] && read -r v <"$z/temp" && read -r n <"$z/type" && printf '%s %s\n' "$v" "$n"
done
fi
"#;

fn script(metrics: MetricSet, interval: Duration) -> String {
    let mut frame =
        String::from("printf '@@uptime\\n'\nread -r u _ </proc/uptime && printf '%s\\n' \"$u\"\n");
    frame.push_str(HOST);
    let sections = [
        (&[MonitorMetric::Cpu, MonitorMetric::Cores][..], STAT),
        (&[MonitorMetric::Memory, MonitorMetric::Swap], MEMINFO),
        (&[MonitorMetric::Load], LOADAVG),
        (&[MonitorMetric::Network], NETDEV),
        (&[MonitorMetric::DiskIo], DISKSTATS),
        (&[MonitorMetric::Disks], DF),
        (&[MonitorMetric::Temperature], TEMP),
    ];
    for (wanted, section) in sections {
        if metrics.any(wanted) {
            frame.push_str(section);
        }
    }

    // One braced group, parsed whole before it runs: nothing it starts can
    // read the rest of the script from standard input.
    let mut script = String::from(
        "{\nexec </dev/null 2>/dev/null\nexport LC_ALL=C\n\
         T=; command -v timeout >/dev/null && T='timeout 5'\nf() {\n",
    );
    script.push_str(&frame);
    // A failed write means the reader is gone, even where SIGPIPE is ignored.
    script.push_str("printf '@@end\\n' || exit\n}\n");
    // A quick second frame gives the first loads and rates within a second.
    let _ = write!(
        script,
        "H=1; f; H=; sleep 1\nwhile :; do f; sleep {}; done\n}}\n",
        interval.as_secs().max(1)
    );
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_requested_sections_are_read() {
        let metrics: MetricSet = [MonitorMetric::Cpu, MonitorMetric::Memory].iter().collect();
        let script = script(metrics, Duration::from_secs(30));
        assert!(script.contains("@@stat"));
        assert!(script.contains("@@meminfo"));
        assert!(!script.contains("@@df"));
        assert!(!script.contains("@@temp"));
        assert!(!script.contains("@@netdev"));
        assert!(script.contains("sleep 30; done"));
    }

    #[cfg(unix)]
    #[test]
    fn the_script_runs_under_sh_and_parses() {
        use std::process::{Command, Stdio};
        if !std::path::Path::new("/proc/stat").exists() {
            return;
        }
        let script = script(MetricSet::all(), Duration::from_secs(1));
        // Two frames: stop the endless loop after the first sleep.
        let script = script.replacen("H=1; f; H=; sleep 1", "H=1; f; H=; f; exit", 1);
        let output = Command::new("sh")
            .arg("-c")
            .arg(&script)
            .stdout(Stdio::piped())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let frames: Vec<Vec<String>> = text
            .split("@@end\n")
            .filter(|frame| !frame.is_empty())
            .map(|frame| frame.lines().map(str::to_owned).collect())
            .collect();
        assert_eq!(frames.len(), 2, "{text}");
        let first = parse(&frames[0]);
        assert!(first.clock.is_some());
        assert!(
            first
                .host
                .as_ref()
                .is_some_and(|host| !host.kernel.is_empty())
        );
        assert!(first.cpu.is_some());
        assert!(!first.cores.as_ref().unwrap().is_empty());
        assert!(first.memory.unwrap().0 > 0);
        assert!(first.load.is_some());
        assert!(first.network.is_some());
        assert!(first.disk_io.is_some());
        assert!(parse(&frames[1]).host.is_none());
    }
}
