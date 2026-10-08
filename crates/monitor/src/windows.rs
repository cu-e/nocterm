//! Windows hosts: a Windows PowerShell 5.1 script reading CIM counters.
//!
//! The script is passed as `-EncodedCommand`, which reads the same from
//! `cmd.exe`, PowerShell or a local spawn, and keeps every frame to a few
//! `key value…` lines so parsing does not depend on the host's locale.
//! Processor, network and disk counters are raw cumulative values, so rates
//! hold across restarts of the script, exactly as on Linux.

use std::{fmt::Write as _, time::Duration};

use base64::Engine as _;
use nocterm_session::ExecRequest;
use nocterm_settings::MonitorMetric;

use crate::{
    CpuTimes, DiskCounters, Filesystem, HostInfo, MetricSet, NetworkCounters, Reading, Temperature,
};

/// The arguments that make PowerShell run `script` and nothing else.
pub(crate) fn powershell(script: &str) -> ExecRequest {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    ExecRequest::new("powershell")
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-EncodedCommand")
        .arg(base64::engine::general_purpose::STANDARD.encode(utf16))
}

/// The watch request for `metrics`, printing a frame every `interval`.
pub(crate) fn request(metrics: MetricSet, interval: Duration) -> ExecRequest {
    powershell(&script(metrics, interval))
}

const PRELUDE: &str = "$ErrorActionPreference='SilentlyContinue'
try{[Console]::OutputEncoding=[Text.Encoding]::UTF8}catch{}
function W($x){try{[Console]::Out.Write($x);[Console]::Out.Flush()}catch{exit}}
$h=1;$q=1
while(1){$l=@('@@win')
$l+=\"clock $([Diagnostics.Stopwatch]::GetTimestamp()/[Diagnostics.Stopwatch]::Frequency)\"
";

const HOST: &str =
    "if($h){$l+=\"hostname $env:COMPUTERNAME\";$l+=\"os $($o.Caption) $($o.Version)\"
$l+=\"cpumodel $((gcim Win32_Processor|select -f 1).Name)\"}
";

const CPU: &str = "foreach($c in gcim Win32_PerfRawData_PerfOS_Processor){$l+=\"cpu $($c.Name) $($c.PercentProcessorTime) $($c.Timestamp_Sys100NS)\"}
";

const MEMORY: &str = "$l+=\"mem $($o.TotalVisibleMemorySize) $($o.FreePhysicalMemory)\"
";

const SWAP: &str = "$l+=\"swap $($o.SizeStoredInPagingFiles) $($o.FreeSpaceInPagingFiles)\"
";

const UPTIME: &str = "$l+=\"up $([long]((Get-Date)-$o.LastBootUpTime).TotalSeconds)\"
";

const NETWORK: &str = "$r=[decimal]0;$s=[decimal]0
foreach($n in gcim Win32_PerfRawData_Tcpip_NetworkInterface){$r+=$n.BytesReceivedPersec;$s+=$n.BytesSentPersec}
$l+=\"net $r $s\"
";

const DISK_IO: &str = "foreach($d in gcim Win32_PerfRawData_PerfDisk_PhysicalDisk -Filter \"Name='_Total'\"){$l+=\"disk $($d.DiskReadBytesPersec) $($d.DiskWriteBytesPersec)\"}
";

const DISKS: &str = "foreach($d in gcim Win32_LogicalDisk -Filter DriveType=3){$l+=\"fs $($d.Size) $($d.FreeSpace) $($d.DeviceID)\"}
";

const TEMPERATURE: &str = "foreach($t in gcim Win32_PerfFormattedData_Counters_ThermalZoneInformation){$l+=\"temp $($t.Temperature) $($t.Name)\"}
";

fn script(metrics: MetricSet, interval: Duration) -> String {
    let mut script = String::from(PRELUDE);
    let system = metrics.any(&[
        MonitorMetric::Memory,
        MonitorMetric::Swap,
        MonitorMetric::Uptime,
    ]);
    script.push_str(if system {
        "$o=gcim Win32_OperatingSystem\n"
    } else {
        "if($h){$o=gcim Win32_OperatingSystem}\n"
    });
    script.push_str(HOST);
    let sections = [
        (&[MonitorMetric::Cpu, MonitorMetric::Cores][..], CPU),
        (&[MonitorMetric::Memory], MEMORY),
        (&[MonitorMetric::Swap], SWAP),
        (&[MonitorMetric::Uptime], UPTIME),
        (&[MonitorMetric::Network], NETWORK),
        (&[MonitorMetric::DiskIo], DISK_IO),
        (&[MonitorMetric::Disks], DISKS),
        (&[MonitorMetric::Temperature], TEMPERATURE),
    ];
    for (wanted, section) in sections {
        if metrics.any(wanted) {
            script.push_str(section);
        }
    }
    let _ = write!(
        script,
        "W(($l -join \"`n\")+\"`n@@end`n\");$h=0\nif($q){{$q=0;sleep 1}}else{{sleep {}}}}}\n",
        interval.as_secs().max(1)
    );
    script
}

/// Turns a frame's lines into a reading.
#[expect(clippy::too_many_lines, reason = "predates the limit")]
pub(crate) fn parse(lines: &[String]) -> Reading {
    let mut reading = Reading::default();
    let mut host = HostInfo::default();
    let mut cores: Vec<(Vec<u64>, CpuTimes)> = Vec::new();
    for line in lines {
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let values: Vec<&str> = rest.split_whitespace().collect();
        let int = |index: usize| values.get(index).and_then(|v| integer(v));
        match key {
            "clock" => reading.clock = rest.trim().parse().ok(),
            "hostname" => host.hostname = rest.trim().to_owned(),
            "os" => host.os = rest.trim().to_owned(),
            "cpumodel" => host.cpu_model = rest.trim().to_owned(),
            "cpu" => {
                let (Some(name), Some(idle), Some(total)) = (values.first(), int(1), int(2)) else {
                    continue;
                };
                let times = CpuTimes {
                    busy: total.saturating_sub(idle),
                    total,
                };
                if *name == "_Total" {
                    reading.cpu = Some(times);
                } else {
                    let order = name
                        .split(',')
                        .filter_map(|part| part.parse().ok())
                        .collect();
                    cores.push((order, times));
                }
            }
            "mem" => {
                if let (Some(total), Some(free)) = (int(0), int(1)) {
                    reading.memory = Some((total * 1024, free.min(total) * 1024));
                }
            }
            "swap" => {
                if let (Some(total), Some(free)) = (int(0), int(1)) {
                    reading.swap = Some((total * 1024, free.min(total) * 1024));
                }
            }
            "up" => reading.uptime = int(0).map(|secs| secs as f64),
            "net" => {
                if let (Some(received), Some(sent)) = (int(0), int(1)) {
                    reading.network = Some(NetworkCounters { received, sent });
                }
            }
            "disk" => {
                if let (Some(read), Some(written)) = (int(0), int(1)) {
                    reading.disk_io = Some(DiskCounters { read, written });
                }
            }
            "fs" => {
                if let (Some(total), Some(free), Some(mount)) = (int(0), int(1), values.get(2)) {
                    reading
                        .filesystems
                        .get_or_insert_default()
                        .push(Filesystem {
                            mount: (*mount).to_owned(),
                            total,
                            used: total.saturating_sub(free),
                        });
                }
            }
            "temp" => {
                if let Some(kelvin) = values.first().and_then(|v| v.parse::<f32>().ok()) {
                    let label = values[1..].join(" ");
                    let celsius = kelvin - 273.15;
                    if kelvin > 0.0 && (-40.0..=150.0).contains(&celsius) {
                        reading
                            .temperatures
                            .get_or_insert_default()
                            .push(Temperature {
                                label: label.trim_start_matches("\\_TZ.").to_owned(),
                                celsius,
                            });
                    }
                }
            }
            _ => {}
        }
    }
    if !host.is_empty() {
        reading.host = Some(host);
    }
    if !cores.is_empty() {
        cores.sort_by(|left, right| left.0.cmp(&right.0));
        reading.cores = Some(cores.into_iter().map(|(_, times)| times).collect());
    }
    reading
}

/// An integer, tolerating a decimal tail from locale-blind arithmetic.
fn integer(text: &str) -> Option<u64> {
    text.split(['.', ',']).next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_fits_a_command_line() {
        let request = request(MetricSet::all(), Duration::from_secs(2));
        // cmd.exe accepts 8191 characters.
        assert!(
            request.command_line().len() < 8000,
            "{}",
            request.command_line().len()
        );
        assert!(request.stdin.is_none());
    }

    #[test]
    fn only_requested_counters_are_queried() {
        let metrics: MetricSet = [MonitorMetric::Cpu].iter().collect();
        let script = script(metrics, Duration::from_secs(30));
        assert!(script.contains("PerfOS_Processor"));
        assert!(!script.contains("LogicalDisk"));
        assert!(!script.contains("NetworkInterface"));
        assert!(script.contains("if($h){$o=gcim"));
        assert!(script.contains("sleep 30}"));
    }

    #[test]
    fn a_frame_is_read() {
        let frame: Vec<String> = "@@win
clock 12345.678
hostname DESKTOP-1
os Microsoft Windows 11 Pro 10.0.22631
cpumodel Intel(R) Core(TM) i7-1185G7
cpu 1 800 1000
cpu 0 900 1000
cpu 10 950 1000
cpu _Total 2650 3000
mem 16000 4000
swap 2000 1500
up 86400
net 1000 2000
disk 300 400
fs 1000 250 C:
temp 310.15 \\_TZ.CPUZ"
            .lines()
            .map(str::to_owned)
            .collect();
        let reading = parse(&frame);
        assert_eq!(reading.clock, Some(12345.678));
        assert_eq!(reading.host.unwrap().hostname, "DESKTOP-1");
        assert_eq!(
            reading.cpu,
            Some(CpuTimes {
                busy: 350,
                total: 3000
            })
        );
        let cores = reading.cores.unwrap();
        assert_eq!(cores[0].busy, 100);
        assert_eq!(cores[1].busy, 200);
        assert_eq!(cores[2].busy, 50);
        assert_eq!(reading.memory, Some((16000 * 1024, 4000 * 1024)));
        assert_eq!(reading.uptime, Some(86400.0));
        assert_eq!(
            reading.network,
            Some(NetworkCounters {
                received: 1000,
                sent: 2000
            })
        );
        assert_eq!(reading.filesystems.unwrap()[0].used, 750);
        let temperature = &reading.temperatures.unwrap()[0];
        assert_eq!(temperature.label, "CPUZ");
        assert!((temperature.celsius - 37.0).abs() < 0.01);
    }
}
