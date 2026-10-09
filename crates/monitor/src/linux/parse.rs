//! Parses one frame of the Linux script.

use std::collections::HashSet;

use crate::{CpuTimes, DiskCounters, Filesystem, HostInfo, NetworkCounters, Reading, Temperature};

/// Interfaces that carry no traffic of their own: loopback, bridges and
/// container plumbing, which would count the same bytes twice.
const VIRTUAL_INTERFACES: &[&str] = &[
    "lo", "veth", "docker", "br-", "virbr", "vnet", "cni", "flannel", "cali", "kube", "tap",
];
/// Block devices that are not physical disks, or that stack on other disks.
const VIRTUAL_DISKS: &[&str] = &["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd"];
/// File systems with nothing on a disk.
const VIRTUAL_FILESYSTEMS: &[&str] = &[
    "tmpfs", "devtmpfs", "overlay", "squashfs", "udev", "none", "shm", "efivarfs", "proc", "sysfs",
];
const VIRTUAL_MOUNTS: &[&str] = &["/proc", "/sys", "/dev", "/run", "/snap", "/var/lib/docker"];

/// Turns a frame's lines into a reading.
#[expect(
    clippy::cognitive_complexity,
    clippy::too_many_lines,
    reason = "predates the limit"
)]
pub(crate) fn parse(lines: &[String]) -> Reading {
    let mut reading = Reading::default();
    let mut section = "";
    let mut host = HostInfo::default();
    let mut cores = Vec::new();
    let mut memory = Memory::default();
    let mut network: Option<NetworkCounters> = None;
    let mut blocks = HashSet::new();
    let mut disks: Vec<(String, DiskCounters)> = Vec::new();
    let mut filesystems: Vec<(String, Filesystem)> = Vec::new();
    let mut temperatures = Vec::new();

    for line in lines {
        if let Some(name) = line.strip_prefix("@@") {
            section = name;
            match section {
                "netdev" => network = Some(NetworkCounters::default()),
                "temp" => reading.temperatures = Some(Vec::new()),
                "df" => reading.filesystems = Some(Vec::new()),
                _ => {}
            }
            continue;
        }
        match section {
            "uptime" => {
                if let Some(up) = number::<f64>(line) {
                    reading.uptime = Some(up);
                    reading.clock = Some(up);
                }
            }
            "host" => host_line(&mut host, line),
            "stat" => {
                if let Some((name, times)) = cpu_line(line) {
                    if name == "cpu" {
                        reading.cpu = Some(times);
                    } else {
                        cores.push(times);
                    }
                }
            }
            "meminfo" => memory.line(line),
            "loadavg" => {
                let values: Vec<f64> = line
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if let [one, five, fifteen] = values[..] {
                    reading.load = Some([one, five, fifteen]);
                }
            }
            "netdev" => {
                if let (Some(total), Some(counters)) = (network.as_mut(), interface_line(line)) {
                    total.received = total.received.saturating_add(counters.received);
                    total.sent = total.sent.saturating_add(counters.sent);
                }
            }
            "blocks" => {
                blocks.insert(line.trim().to_owned());
            }
            "diskstats" => {
                if let Some(disk) = disk_line(line) {
                    disks.push(disk);
                }
            }
            "df" => {
                if let Some(filesystem) = df_line(line) {
                    filesystems.push(filesystem);
                }
            }
            "temp" => {
                if let Some(temperature) = temperature_line(line) {
                    temperatures.push(temperature);
                }
            }
            _ => {}
        }
    }

    if !host.is_empty() {
        reading.host = Some(host);
    }
    if !cores.is_empty() {
        reading.cores = Some(cores);
    }
    reading.memory = memory.memory();
    reading.swap = memory.swap();
    reading.network = network;
    if lines.iter().any(|line| line == "@@diskstats") {
        let mut total = DiskCounters::default();
        for (name, counters) in disks {
            if physical_disk(&name, &blocks) {
                total.read = total.read.saturating_add(counters.read);
                total.written = total.written.saturating_add(counters.written);
            }
        }
        reading.disk_io = Some(total);
    }
    if let Some(list) = reading.filesystems.as_mut() {
        let mut sources = HashSet::new();
        for (source, filesystem) in filesystems {
            if sources.insert(source) {
                list.push(filesystem);
            }
        }
    }
    if let Some(list) = reading.temperatures.as_mut() {
        *list = temperatures;
    }
    reading
}

fn number<T: std::str::FromStr>(text: &str) -> Option<T> {
    text.split_whitespace().next()?.parse().ok()
}

fn host_line(host: &mut HostInfo, line: &str) {
    let Some((key, value)) = line.split_once(' ') else {
        return;
    };
    let value = value.trim().trim_matches(['"', '\'']).to_owned();
    match key {
        "hostname" => host.hostname = value,
        "kernel" => host.kernel = value,
        "os" => host.os = value,
        "cpu" if host.cpu_model.is_empty() => host.cpu_model = value,
        _ => {}
    }
}

/// `cpu  user nice system idle iowait irq softirq steal …`
fn cpu_line(line: &str) -> Option<(&str, CpuTimes)> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let values: Vec<u64> = fields.take(8).filter_map(|v| v.parse().ok()).collect();
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle = values[3] + values.get(4).copied().unwrap_or_default();
    Some((
        name,
        CpuTimes {
            busy: total.saturating_sub(idle),
            total,
        },
    ))
}

#[derive(Default)]
struct Memory {
    total: Option<u64>,
    free: Option<u64>,
    available: Option<u64>,
    buffers: u64,
    cached: u64,
    swap_total: Option<u64>,
    swap_free: Option<u64>,
}

impl Memory {
    fn line(&mut self, line: &str) {
        let Some((key, value)) = line.split_once(' ') else {
            return;
        };
        let Some(kib) = number::<u64>(value) else {
            return;
        };
        let bytes = kib.saturating_mul(1024);
        match key {
            "MemTotal:" => self.total = Some(bytes),
            "MemFree:" => self.free = Some(bytes),
            "MemAvailable:" => self.available = Some(bytes),
            "Buffers:" => self.buffers = bytes,
            "Cached:" => self.cached = bytes,
            "SwapTotal:" => self.swap_total = Some(bytes),
            "SwapFree:" => self.swap_free = Some(bytes),
            _ => {}
        }
    }

    fn memory(&self) -> Option<(u64, u64)> {
        let total = self.total?;
        // Kernels before 3.14 have no MemAvailable: estimate it.
        let available = self
            .available
            .or_else(|| Some(self.free? + self.buffers + self.cached))?;
        Some((total, available.min(total)))
    }

    fn swap(&self) -> Option<(u64, u64)> {
        let total = self.swap_total?;
        Some((total, self.swap_free.unwrap_or(total).min(total)))
    }
}

/// `eth0: rx_bytes packets … tx_bytes …`; the colon may touch the number.
fn interface_line(line: &str) -> Option<NetworkCounters> {
    let (name, counters) = line.split_once(':')?;
    let name = name.trim();
    if VIRTUAL_INTERFACES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return None;
    }
    let values: Vec<u64> = counters
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    Some(NetworkCounters {
        received: *values.first()?,
        sent: *values.get(8)?,
    })
}

/// `major minor name reads merged sectors_read ms writes merged sectors_written …`
fn disk_line(line: &str) -> Option<(String, DiskCounters)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let sectors = |index: usize| fields.get(index)?.parse::<u64>().ok();
    // Sectors in diskstats are always 512 bytes, whatever the device's own size.
    Some((
        (*fields.get(2)?).to_owned(),
        DiskCounters {
            read: sectors(5)?.saturating_mul(512),
            written: sectors(9)?.saturating_mul(512),
        },
    ))
}

/// Whole physical disks only: partitions would count their disk twice.
fn physical_disk(name: &str, blocks: &HashSet<String>) -> bool {
    if VIRTUAL_DISKS.iter().any(|prefix| name.starts_with(prefix)) {
        return false;
    }
    if !blocks.is_empty() {
        return blocks.contains(name);
    }
    // Without /sys: whole disks by their usual names.
    let letters = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase()))
    };
    let numbered = |prefix: &str, infix: Option<char>| {
        name.strip_prefix(prefix).is_some_and(|rest| match infix {
            Some(infix) => rest
                .split_once(infix)
                .is_some_and(|(a, b)| digits(a) && digits(b)),
            None => digits(rest),
        })
    };
    letters("sd")
        || letters("vd")
        || letters("xvd")
        || letters("hd")
        || numbered("nvme", Some('n'))
        || numbered("mmcblk", None)
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// `source 1024-blocks used available capacity mount…`
fn df_line(line: &str) -> Option<(String, Filesystem)> {
    let mut fields = line.split_whitespace();
    let source = fields.next()?;
    let size: u64 = fields.next()?.parse().ok()?;
    let _used: u64 = fields.next()?.parse().ok()?;
    let available: u64 = fields.next()?.parse().ok()?;
    let _capacity = fields.next()?;
    let mount = fields.collect::<Vec<_>>().join(" ");
    let virtual_source = VIRTUAL_FILESYSTEMS.contains(&source) || source.starts_with("/dev/loop");
    let virtual_mount = VIRTUAL_MOUNTS
        .iter()
        .any(|prefix| mount == *prefix || mount.starts_with(&format!("{prefix}/")));
    if size == 0 || mount.is_empty() || virtual_source || virtual_mount {
        return None;
    }
    let total = size.saturating_mul(1024);
    // Space reserved for root counts as used, as df's own percentage does.
    let used = total.saturating_sub(available.saturating_mul(1024));
    Some((source.to_owned(), Filesystem { mount, total, used }))
}

/// `millidegrees name [label…]`
fn temperature_line(line: &str) -> Option<Temperature> {
    let (value, label) = line.split_once(' ')?;
    let celsius = value.parse::<f32>().ok()? / 1000.0;
    if !(-40.0..=150.0).contains(&celsius) || celsius == 0.0 {
        return None;
    }
    Some(Temperature {
        label: label.split_whitespace().collect::<Vec<_>>().join(" "),
        celsius,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    const FRAME: &str = "@@uptime
3600.50
@@host
hostname box
kernel 6.8.0
os \"Ubuntu 24.04 LTS\"
cpu  AMD Ryzen 7 5800X
@@stat
cpu  100 0 100 700 100 0 0 0 0 0
cpu0 50 0 50 350 50 0 0 0 0 0
cpu1 50 0 50 350 50 0 0 0 0 0
@@meminfo
MemTotal: 16000
MemFree: 2000
MemAvailable: 8000
SwapTotal: 4000
SwapFree: 3000
@@loadavg
0.50 0.25 0.10
@@netdev
lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0
eth0:1000 5 0 0 0 0 0 0 2000 5 0 0 0 0 0 0
wlan0: 10 1 0 0 0 0 0 0 20 1 0 0 0 0 0 0
docker0: 77 1 0 0 0 0 0 0 77 1 0 0 0 0 0 0
@@blocks
sda
loop0
@@diskstats
   8       0 sda 10 0 100 0 20 0 200 0 0 0 0
   8       1 sda1 10 0 100 0 20 0 200 0 0 0 0
   7       0 loop0 1 0 50 0 0 0 0 0 0 0 0
@@df
Filesystem     1024-blocks      Used Available Capacity Mounted on
/dev/sda1         1000       600       300      67% /
/dev/sda1         1000       600       300      67% /home
tmpfs              100        10        90      10% /run
/dev/sdb1         2000       100      1900       5% /mnt/My Data
@@temp
45000 coretemp Package id 0
38500 nvme Composite
0 acpitz
";

    #[test]
    fn a_full_frame_is_read() {
        let reading = parse(&lines(FRAME));
        assert_eq!(reading.clock, Some(3600.5));
        let host = reading.host.unwrap();
        assert_eq!(host.os, "Ubuntu 24.04 LTS");
        assert_eq!(host.cpu_model, "AMD Ryzen 7 5800X");
        assert_eq!(
            reading.cpu,
            Some(CpuTimes {
                busy: 200,
                total: 1000
            })
        );
        assert_eq!(reading.cores.unwrap().len(), 2);
        assert_eq!(reading.memory, Some((16000 * 1024, 8000 * 1024)));
        assert_eq!(reading.swap, Some((4000 * 1024, 3000 * 1024)));
        assert_eq!(reading.load, Some([0.5, 0.25, 0.1]));
        assert_eq!(
            reading.network,
            Some(NetworkCounters {
                received: 1010,
                sent: 2020
            })
        );
        assert_eq!(
            reading.disk_io,
            Some(DiskCounters {
                read: 100 * 512,
                written: 200 * 512
            })
        );
        let filesystems = reading.filesystems.unwrap();
        assert_eq!(filesystems.len(), 2);
        assert_eq!(filesystems[0].mount, "/");
        assert_eq!(filesystems[0].used, 700 * 1024);
        assert_eq!(filesystems[1].mount, "/mnt/My Data");
        let temperatures = reading.temperatures.unwrap();
        assert_eq!(temperatures.len(), 2);
        assert_eq!(temperatures[0].label, "coretemp Package id 0");
        assert_eq!(temperatures[0].celsius, 45.0);
    }

    #[test]
    fn missing_sections_stay_unknown() {
        let reading = parse(&lines(
            "@@uptime\n12.0\n@@meminfo\nMemTotal: 100\nMemFree: 10\nCached: 20\n",
        ));
        assert_eq!(reading.memory, Some((102_400, 30_720)));
        assert_eq!(reading.cpu, None);
        assert_eq!(reading.network, None);
        assert_eq!(reading.filesystems, None);
        assert_eq!(reading.host, None);
    }

    #[test]
    fn whole_disks_are_recognised_without_sys() {
        let none = HashSet::new();
        for disk in ["sda", "vdb", "xvda", "nvme0n1", "mmcblk0"] {
            assert!(physical_disk(disk, &none), "{disk}");
        }
        for partition in ["sda1", "nvme0n1p2", "mmcblk0p1", "dm-0", "loop3", "zram0"] {
            assert!(!physical_disk(partition, &none), "{partition}");
        }
    }
}
