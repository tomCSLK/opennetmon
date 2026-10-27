//! Sensors for the machine opennetmon runs on ("Dieser Server"): CPU, load, memory, uptime,
//! file systems, temperatures and network traffic, read from `/proc`, `/sys` and `statvfs`.
//! No SNMP and no credentials; everything is read-only. The systemd unit must leave the system
//! files in `/proc` visible (no `ProcSubset=pid`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::counter::{CounterReading, CounterTracker, CounterWidth, Rate};
use super::spec::ChannelDef;
use super::{ChannelReading, Measurement, PollContext, PollFuture, Sensor, SensorError, Unit};
use crate::i18n::Msg;
use crate::status::Thresholds;

/// Longest accepted mount point, sensor key or interface name.
pub const MAX_TARGET_CHARS: usize = 512;
/// `statvfs` of a hanging network mount must not block a poll for long.
const STATVFS_TIMEOUT: Duration = Duration::from_secs(5);

/// File systems worth watching; pseudo file systems (proc, tmpfs, cgroup …) and read-only images
/// such as snaps (squashfs) are left out.
const DISK_TYPES: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "zfs", "f2fs", "bcachefs", "jfs", "reiserfs", "vfat",
    "exfat", "ntfs", "ntfs3", "fuseblk", "nfs", "nfs4", "cifs", "smb3",
];

/// What a local sensor measures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalMetric {
    Cpu,
    Load,
    Memory,
    Uptime,
    /// A file system, by mount point.
    Disk {
        mount: String,
    },
    /// A hardware temperature, by its key from [`temperatures`] (`chip/label`).
    Temperature {
        sensor: String,
    },
    /// A network interface, by name.
    Traffic {
        interface: String,
    },
}

impl LocalMetric {
    /// The value stored in `sensors.target`, e.g. `cpu`, `disk:/var` or `net:eth0`.
    pub fn target(&self) -> String {
        match self {
            Self::Cpu => "cpu".to_owned(),
            Self::Load => "load".to_owned(),
            Self::Memory => "memory".to_owned(),
            Self::Uptime => "uptime".to_owned(),
            Self::Disk { mount } => format!("disk:{mount}"),
            Self::Temperature { sensor } => format!("temp:{sensor}"),
            Self::Traffic { interface } => format!("net:{interface}"),
        }
    }

    /// Inverse of [`Self::target`].
    pub fn parse(target: &str) -> Option<Self> {
        Some(match target {
            "cpu" => Self::Cpu,
            "load" => Self::Load,
            "memory" => Self::Memory,
            "uptime" => Self::Uptime,
            other => match other.split_once(':')? {
                ("disk", mount) => Self::Disk {
                    mount: mount.to_owned(),
                },
                ("temp", sensor) => Self::Temperature {
                    sensor: sensor.to_owned(),
                },
                ("net", interface) => Self::Traffic {
                    interface: interface.to_owned(),
                },
                _ => return None,
            },
        })
    }

    pub fn validate(&self) -> Result<(), Msg> {
        let text_ok = |s: &str| {
            !s.trim().is_empty()
                && s.chars().count() <= MAX_TARGET_CHARS
                && !s.chars().any(char::is_control)
        };
        let ok = match self {
            Self::Cpu | Self::Load | Self::Memory | Self::Uptime => true,
            Self::Disk { mount } => mount.starts_with('/') && text_ok(mount),
            Self::Temperature { sensor } => text_ok(sensor),
            // Linux interface names: at most 15 bytes, no slash, no whitespace.
            Self::Traffic { interface } => {
                text_ok(interface)
                    && interface.len() <= 15
                    && !interface.contains(['/', ' ', ':'])
                    && interface != "."
                    && interface != ".."
            }
        };
        if ok {
            Ok(())
        } else {
            Err(Msg::new("local-target-invalid"))
        }
    }

    pub fn channels(&self) -> Vec<ChannelDef> {
        let def = |key, unit| ChannelDef { key, unit };
        match self {
            Self::Cpu => vec![def("total", Unit::Percent), def("iowait", Unit::Percent)],
            Self::Load => vec![
                def("load1", Unit::Count),
                def("load5", Unit::Count),
                def("load15", Unit::Count),
            ],
            Self::Memory => vec![
                def("used", Unit::Percent),
                def("available", Unit::Bytes),
                def("swap_used", Unit::Percent),
            ],
            Self::Uptime => vec![def("uptime", Unit::Seconds)],
            Self::Disk { .. } => vec![def("used", Unit::Percent), def("free", Unit::Bytes)],
            Self::Temperature { .. } => vec![def("temperature", Unit::Celsius)],
            Self::Traffic { .. } => vec![
                def("in", Unit::BitsPerSecond),
                def("out", Unit::BitsPerSecond),
            ],
        }
    }

    /// Limits set when the sensor is created: memory and file systems warn when nearly full.
    /// CPU and load have none (short peaks are normal); temperatures take the hardware's own
    /// limits from the creation form.
    pub fn default_limits(&self) -> Vec<(&'static str, Thresholds)> {
        let above = |warn, down| Thresholds {
            warn_above: Some(warn),
            down_above: Some(down),
            ..Thresholds::default()
        };
        match self {
            Self::Memory => vec![("used", above(90.0, 95.0))],
            Self::Disk { .. } => vec![("used", above(85.0, 95.0))],
            _ => Vec::new(),
        }
    }
}

/// Where the system files are; tests point this at a directory of their own.
#[derive(Debug, Clone)]
pub struct Paths {
    pub proc: PathBuf,
    pub sys: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            proc: PathBuf::from("/proc"),
            sys: PathBuf::from("/sys"),
        }
    }
}

fn read(path: &Path) -> Result<String, SensorError> {
    std::fs::read_to_string(path)
        .map_err(|e| SensorError::BadValue(format!("cannot read {}: {e}", path.display())))
}

fn bad(what: &str) -> SensorError {
    SensorError::BadValue(format!("unexpected content of {what}"))
}

fn reading(key: &str, unit: Unit, value: Option<f64>) -> ChannelReading {
    ChannelReading {
        key: key.to_owned(),
        unit,
        value,
    }
}

/// Cumulative CPU times from the first line of `/proc/stat` (in clock ticks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    total: u64,
    idle: u64,
    iowait: u64,
}

fn parse_cpu(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    // user nice system idle iowait irq softirq steal (guest times are part of user/nice).
    let base = fields.get(..8)?;
    Some(CpuTimes {
        total: base.iter().sum(),
        idle: base[3] + base[4],
        iowait: base[4],
    })
}

/// Busy and I/O wait share between two readings in percent; `None` if no time passed.
fn cpu_usage(previous: CpuTimes, current: CpuTimes) -> Option<(f64, f64)> {
    let total = current.total.checked_sub(previous.total)?;
    let idle = current.idle.checked_sub(previous.idle)?;
    let iowait = current.iowait.checked_sub(previous.iowait)?;
    if total == 0 || idle > total {
        return None;
    }
    let pct = |part: u64| part as f64 * 100.0 / total as f64;
    Some((pct(total - idle), pct(iowait)))
}

fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut parts = text.split_whitespace().map(str::parse::<f64>);
    let mut next = || parts.next()?.ok().filter(|v| v.is_finite());
    Some([next()?, next()?, next()?])
}

/// The first number of `/proc/uptime`, in seconds.
fn parse_uptime(text: &str) -> Option<f64> {
    text.split_whitespace()
        .next()?
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite())
}

/// Memory figures from `/proc/meminfo`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Memory {
    total: f64,
    available: f64,
    swap_total: f64,
    swap_free: f64,
}

fn parse_meminfo(text: &str) -> Option<Memory> {
    let fields: HashMap<&str, f64> = text
        .lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let kib: f64 = rest.split_whitespace().next()?.parse().ok()?;
            Some((name, kib * 1024.0))
        })
        .collect();
    Some(Memory {
        total: *fields.get("MemTotal")?,
        available: *fields.get("MemAvailable")?,
        swap_total: fields.get("SwapTotal").copied().unwrap_or(0.0),
        swap_free: fields.get("SwapFree").copied().unwrap_or(0.0),
    })
}

impl Memory {
    fn readings(&self) -> Vec<ChannelReading> {
        let used = if self.total > 0.0 {
            Some((self.total - self.available).max(0.0) * 100.0 / self.total)
        } else {
            None
        };
        let swap_used = if self.swap_total > 0.0 {
            (self.swap_total - self.swap_free).max(0.0) * 100.0 / self.swap_total
        } else {
            0.0
        };
        vec![
            reading("used", Unit::Percent, used),
            reading("available", Unit::Bytes, Some(self.available)),
            reading("swap_used", Unit::Percent, Some(swap_used)),
        ]
    }
}

/// A mounted file system from `/proc/self/mountinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filesystem {
    pub mount: String,
    pub fstype: String,
    pub source: String,
}

/// Undoes the octal escapes of mountinfo (`\040` = space).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && let Some(code) = field
                .get(i + 1..i + 4)
                .and_then(|o| u8::from_str_radix(o, 8).ok())
        {
            out.push(code);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Real file systems, each device once (bind mounts such as systemd's read-only views of `/`
/// share the device number; the shortest mount point wins), sorted by mount point.
fn parse_mountinfo(text: &str) -> Vec<Filesystem> {
    let mut by_device: HashMap<String, Filesystem> = HashMap::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let left: Vec<&str> = left.split(' ').collect();
        let right: Vec<&str> = right.split(' ').collect();
        let (Some(device), Some(root), Some(mount), Some(fstype)) =
            (left.get(2), left.get(3), left.get(4), right.first())
        else {
            continue;
        };
        if *root != "/" || !DISK_TYPES.contains(fstype) {
            continue;
        }
        let fs = Filesystem {
            mount: unescape(mount),
            fstype: (*fstype).to_owned(),
            source: right.get(1).map(|s| unescape(s)).unwrap_or_default(),
        };
        match by_device.get(*device) {
            Some(known) if known.mount.len() <= fs.mount.len() => {}
            _ => {
                by_device.insert((*device).to_owned(), fs);
            }
        }
    }
    let mut list: Vec<Filesystem> = by_device.into_values().collect();
    list.sort_by(|a, b| a.mount.cmp(&b.mount));
    list
}

pub fn filesystems(paths: &Paths) -> Vec<Filesystem> {
    std::fs::read_to_string(paths.proc.join("self/mountinfo"))
        .map(|text| parse_mountinfo(&text))
        .unwrap_or_default()
}

/// Size and use of a file system, as `df` shows it (reserved blocks count as used).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiskUsage {
    pub size: f64,
    pub used_pct: f64,
    pub free: f64,
}

pub async fn disk_usage(mount: &str) -> Result<DiskUsage, SensorError> {
    let path = PathBuf::from(mount);
    let task = tokio::task::spawn_blocking(move || rustix::fs::statvfs(&path));
    let stat = tokio::time::timeout(STATVFS_TIMEOUT, task)
        .await
        .map_err(|_| SensorError::BadValue(format!("{mount} does not answer")))?
        .map_err(|e| SensorError::BadValue(e.to_string()))?
        .map_err(|e| SensorError::BadValue(format!("cannot read {mount}: {e}")))?;
    let unit = stat.f_frsize as f64;
    let used = stat.f_blocks.saturating_sub(stat.f_bfree) as f64;
    let available = stat.f_bavail as f64;
    if stat.f_blocks == 0 || used + available <= 0.0 {
        return Err(SensorError::BadValue(format!("{mount} reports no size")));
    }
    Ok(DiskUsage {
        size: stat.f_blocks as f64 * unit,
        used_pct: used * 100.0 / (used + available),
        free: available * unit,
    })
}

/// One temperature input of a hardware monitor (`/sys/class/hwmon`).
#[derive(Debug, Clone, PartialEq)]
pub struct Temperature {
    /// Stable key: chip name (with `#2`, `#3` … for further chips of the same name) and label.
    pub key: String,
    pub celsius: Option<f64>,
    /// The hardware's own limits, if it reports plausible ones.
    pub max: Option<f64>,
    pub crit: Option<f64>,
    input: PathBuf,
}

fn millidegrees(path: &Path) -> Option<f64> {
    let raw: f64 = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    Some(raw / 1000.0).filter(|c| c.is_finite() && *c > -100.0 && *c < 250.0)
}

/// All temperature inputs, ordered by chip and input number. hwmon numbers change between
/// boots, so the key is built from names only.
pub fn temperatures(paths: &Paths) -> Vec<Temperature> {
    let Ok(entries) = std::fs::read_dir(paths.sys.join("class/hwmon")) else {
        return Vec::new();
    };
    let mut chips: Vec<(u32, PathBuf)> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let n = e
                .file_name()
                .to_str()?
                .strip_prefix("hwmon")?
                .parse()
                .ok()?;
            Some((n, e.path()))
        })
        .collect();
    chips.sort();
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut found = Vec::new();
    for (_, dir) in chips {
        let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
            continue;
        };
        let name = name.trim().to_owned();
        let count = seen.entry(name.clone()).or_insert(0);
        *count += 1;
        let chip = if *count == 1 {
            name
        } else {
            format!("{name}#{count}")
        };
        let mut inputs: Vec<u32> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let file = e.ok()?.file_name().into_string().ok()?;
                file.strip_prefix("temp")?
                    .strip_suffix("_input")?
                    .parse()
                    .ok()
            })
            .collect();
        inputs.sort_unstable();
        for n in inputs {
            let label = std::fs::read_to_string(dir.join(format!("temp{n}_label")))
                .ok()
                .map(|l| l.trim().chars().filter(|c| !c.is_control()).collect())
                .filter(|l: &String| !l.is_empty())
                .unwrap_or_else(|| format!("temp{n}"));
            let input = dir.join(format!("temp{n}_input"));
            let max = millidegrees(&dir.join(format!("temp{n}_max"))).filter(|v| *v > 0.0);
            let crit = millidegrees(&dir.join(format!("temp{n}_crit"))).filter(|v| *v > 0.0);
            found.push(Temperature {
                key: format!("{chip}/{label}"),
                celsius: millidegrees(&input),
                max,
                crit,
                input,
            });
        }
    }
    found
}

impl Temperature {
    /// Warning at the hardware's maximum, down at its critical limit (if it reports them).
    pub fn limits(&self) -> Thresholds {
        let down = self.crit;
        let warn = self.max.filter(|m| down.is_none_or(|d| *m < d));
        Thresholds {
            warn_above: warn,
            down_above: down,
            ..Thresholds::default()
        }
    }
}

/// A network interface from `/sys/class/net` (without loopback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetInterface {
    pub name: String,
    /// `operstate`, e.g. `up`, `down`, `unknown`.
    pub state: String,
    /// Backed by hardware (not a bridge, VPN or container interface).
    pub physical: bool,
}

pub fn interfaces(paths: &Paths) -> Vec<NetInterface> {
    let dir = paths.sys.join("class/net");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut list: Vec<NetInterface> = entries
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            if name == "lo" {
                return None;
            }
            let base = dir.join(&name);
            Some(NetInterface {
                state: std::fs::read_to_string(base.join("operstate"))
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default(),
                physical: base.join("device").exists(),
                name,
            })
        })
        .collect();
    list.sort_by(|a, b| crate::ordering::natural_cmp(&a.name, &b.name));
    list
}

/// The machine's host name, to show where the values come from.
pub fn hostname(paths: &Paths) -> String {
    std::fs::read_to_string(paths.proc.join("sys/kernel/hostname"))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

pub struct LocalSensor {
    metric: LocalMetric,
    paths: Paths,
    cpu: Option<CpuTimes>,
    rx: CounterTracker,
    tx: CounterTracker,
}

impl LocalSensor {
    pub fn new(metric: LocalMetric, paths: Paths) -> Self {
        Self {
            metric,
            paths,
            cpu: None,
            rx: CounterTracker::new(CounterWidth::Bits64, None),
            tx: CounterTracker::new(CounterWidth::Bits64, None),
        }
    }

    /// One reading outside the poller, e.g. to show the current value before a sensor exists.
    pub async fn read_once(&mut self) -> Result<Vec<ChannelReading>, SensorError> {
        self.measure(0).await
    }

    async fn measure(&mut self, now_ms: i64) -> Result<Vec<ChannelReading>, SensorError> {
        let proc = |file: &str| self.paths.proc.join(file);
        Ok(match &self.metric {
            LocalMetric::Cpu => {
                let current = parse_cpu(&read(&proc("stat"))?).ok_or_else(|| bad("/proc/stat"))?;
                let usage = self
                    .cpu
                    .replace(current)
                    .and_then(|p| cpu_usage(p, current));
                vec![
                    reading("total", Unit::Percent, usage.map(|u| u.0)),
                    reading("iowait", Unit::Percent, usage.map(|u| u.1)),
                ]
            }
            LocalMetric::Load => {
                let [l1, l5, l15] =
                    parse_loadavg(&read(&proc("loadavg"))?).ok_or_else(|| bad("/proc/loadavg"))?;
                vec![
                    reading("load1", Unit::Count, Some(l1)),
                    reading("load5", Unit::Count, Some(l5)),
                    reading("load15", Unit::Count, Some(l15)),
                ]
            }
            LocalMetric::Memory => parse_meminfo(&read(&proc("meminfo"))?)
                .ok_or_else(|| bad("/proc/meminfo"))?
                .readings(),
            LocalMetric::Uptime => {
                let up =
                    parse_uptime(&read(&proc("uptime"))?).ok_or_else(|| bad("/proc/uptime"))?;
                vec![reading("uptime", Unit::Seconds, Some(up))]
            }
            LocalMetric::Disk { mount } => {
                let usage = disk_usage(mount).await?;
                vec![
                    reading("used", Unit::Percent, Some(usage.used_pct)),
                    reading("free", Unit::Bytes, Some(usage.free)),
                ]
            }
            LocalMetric::Temperature { sensor } => {
                let input = temperatures(&self.paths)
                    .into_iter()
                    .find(|t| t.key == *sensor)
                    .ok_or_else(|| {
                        SensorError::BadValue(format!("temperature sensor {sensor} not found"))
                    })?;
                let celsius = input.celsius.ok_or_else(|| {
                    SensorError::BadValue(format!("{} gives no temperature", input.input.display()))
                })?;
                vec![reading("temperature", Unit::Celsius, Some(celsius))]
            }
            LocalMetric::Traffic { interface } => {
                let stats = self.paths.sys.join("class/net").join(interface);
                if !stats.exists() {
                    return Err(SensorError::BadValue(format!(
                        "network interface {interface} not found"
                    )));
                }
                let counter = |file: &str| -> Result<u64, SensorError> {
                    let path = stats.join("statistics").join(file);
                    read(&path)?
                        .trim()
                        .parse()
                        .map_err(|_| bad(&path.display().to_string()))
                };
                let (rx, tx) = (counter("rx_bytes")?, counter("tx_bytes")?);
                let bits = |rate: Rate| match rate {
                    Rate::PerSecond(bytes) => Some(bytes * 8.0),
                    Rate::First | Rate::Skipped(_) => None,
                };
                let at = |value| CounterReading {
                    value,
                    at_ms: now_ms,
                    uptime_ticks: None,
                };
                vec![
                    reading("in", Unit::BitsPerSecond, bits(self.rx.update(at(rx)))),
                    reading("out", Unit::BitsPerSecond, bits(self.tx.update(at(tx)))),
                ]
            }
        })
    }
}

impl Sensor for LocalSensor {
    fn poll<'a>(&'a mut self, ctx: &'a mut PollContext<'_>) -> PollFuture<'a> {
        Box::pin(async move {
            Ok(Measurement {
                note: None,
                channels: self.measure(ctx.now_ms).await?,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::ping::PingClients;

    /// A fake `/proc` and `/sys` in a temporary directory.
    struct Fake {
        dir: tempfile::TempDir,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn paths(&self) -> Paths {
            Paths {
                proc: self.dir.path().join("proc"),
                sys: self.dir.path().join("sys"),
            }
        }

        fn write(&self, file: &str, content: &str) {
            let path = self.dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }

    async fn poll(sensor: &mut LocalSensor, now_ms: i64) -> Result<Measurement, SensorError> {
        let ping = PingClients::new();
        let mut ctx = PollContext {
            host: "127.0.0.1",
            now_ms,
            timeout: Duration::from_secs(1),
            ping: &ping,
            snmp: None,
        };
        sensor.poll(&mut ctx).await
    }

    fn values(m: &Measurement) -> Vec<(&str, Option<f64>)> {
        m.channels
            .iter()
            .map(|c| (c.key.as_str(), c.value))
            .collect()
    }

    #[test]
    fn targets_round_trip_and_are_checked() {
        for metric in [
            LocalMetric::Cpu,
            LocalMetric::Load,
            LocalMetric::Memory,
            LocalMetric::Uptime,
            LocalMetric::Disk {
                mount: "/srv/daten 2".into(),
            },
            LocalMetric::Temperature {
                sensor: "coretemp/Package id 0".into(),
            },
            LocalMetric::Traffic {
                interface: "enp0s31f6".into(),
            },
        ] {
            assert_eq!(LocalMetric::parse(&metric.target()), Some(metric.clone()));
            assert!(metric.validate().is_ok(), "{metric:?}");
        }
        assert_eq!(LocalMetric::parse("disk"), None);
        assert_eq!(LocalMetric::parse("gpu"), None);
        for bad in [
            LocalMetric::Disk {
                mount: "var".into(),
            },
            LocalMetric::Disk {
                mount: "/a\nb".into(),
            },
            LocalMetric::Temperature { sensor: " ".into() },
            LocalMetric::Traffic {
                interface: "../etc".into(),
            },
            LocalMetric::Traffic {
                interface: "x".repeat(16),
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn proc_files_are_parsed() {
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4 5 6 7 8 0 0\n";
        let a = parse_cpu(stat).unwrap();
        assert_eq!((a.total, a.idle, a.iowait), (1000, 850, 50));
        let b = parse_cpu("cpu  200 0 100 1600 100 0 0 0 0 0\n").unwrap();
        // 1000 ticks passed: 150 busy, 50 waiting for I/O.
        assert_eq!(cpu_usage(a, b), Some((15.0, 5.0)));
        assert_eq!(cpu_usage(a, a), None, "no time passed");
        assert_eq!(cpu_usage(b, a), None, "counters went back");
        assert_eq!(parse_cpu("cpu  1 2 3\n"), None);

        assert_eq!(
            parse_loadavg("0.39 0.46 0.54 1/1047 131193\n"),
            Some([0.39, 0.46, 0.54])
        );
        assert_eq!(parse_loadavg("0.39 x"), None);
        assert_eq!(parse_uptime("54250.70 304020.39\n"), Some(54250.7));

        let mem = parse_meminfo(
            "MemTotal:       1000 kB\nMemFree: 100 kB\nMemAvailable:    250 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        )
        .unwrap();
        let r = mem.readings();
        assert_eq!(r[0].value, Some(75.0));
        assert_eq!(r[1].value, Some(250.0 * 1024.0));
        assert_eq!(r[2].value, Some(0.0), "no swap counts as unused");
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), None);
    }

    #[test]
    fn mountinfo_lists_each_real_file_system_once() {
        let text = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
23 22 0:5 / /proc rw - proc proc rw
24 22 259:1 / /boot/efi rw - vfat /dev/nvme0n1p1 rw
25 22 7:3 / /snap/core/1 ro - squashfs /dev/loop3 ro
26 22 259:2 / /var/lib/opennetmon rw - ext4 /dev/nvme0n1p2 rw
27 22 259:2 /home/x /mnt/bind rw - ext4 /dev/nvme0n1p2 rw
28 22 0:40 / /srv/mein\\040nas rw - nfs4 nas:/export rw
29 22 0:41 / /run rw - tmpfs tmpfs rw
";
        let list = parse_mountinfo(text);
        let mounts: Vec<_> = list.iter().map(|f| f.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/boot/efi", "/srv/mein nas"]);
        assert_eq!(list[0].source, "/dev/nvme0n1p2");
        assert_eq!(list[2].fstype, "nfs4");
    }

    #[tokio::test]
    async fn the_root_file_system_has_a_size() {
        let usage = disk_usage("/").await.unwrap();
        assert!(usage.size > 0.0 && usage.free >= 0.0);
        assert!((0.0..=100.0).contains(&usage.used_pct));
        assert!(disk_usage("/does/not/exist").await.is_err());
    }

    #[test]
    fn temperatures_get_stable_keys_and_plausible_limits() {
        let fake = Fake::new();
        fake.write("sys/class/hwmon/hwmon0/name", "AC\n");
        fake.write("sys/class/hwmon/hwmon2/name", "coretemp\n");
        fake.write("sys/class/hwmon/hwmon2/temp1_input", "45000\n");
        fake.write("sys/class/hwmon/hwmon2/temp1_label", "Package id 0\n");
        fake.write("sys/class/hwmon/hwmon2/temp1_max", "100000\n");
        fake.write("sys/class/hwmon/hwmon2/temp1_crit", "100000\n");
        fake.write("sys/class/hwmon/hwmon2/temp2_input", "41500\n");
        fake.write("sys/class/hwmon/hwmon10/name", "nvme\n");
        fake.write("sys/class/hwmon/hwmon10/temp1_input", "38850\n");
        fake.write("sys/class/hwmon/hwmon10/temp1_max", "84850\n");
        fake.write("sys/class/hwmon/hwmon11/name", "nvme\n");
        fake.write("sys/class/hwmon/hwmon11/temp1_input", "garbage\n");
        let found = temperatures(&fake.paths());
        let keys: Vec<_> = found.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "coretemp/Package id 0",
                "coretemp/temp2",
                "nvme/temp1",
                "nvme#2/temp1"
            ]
        );
        assert_eq!(found[0].celsius, Some(45.0));
        assert_eq!(found[3].celsius, None);
        // max equal to crit: only the down limit.
        assert_eq!(
            (found[0].limits().warn_above, found[0].limits().down_above),
            (None, Some(100.0))
        );
        assert_eq!(found[2].limits().warn_above, Some(84.85));
    }

    #[test]
    fn interfaces_leave_out_loopback() {
        let fake = Fake::new();
        fake.write("sys/class/net/lo/operstate", "unknown\n");
        fake.write("sys/class/net/eth10/operstate", "down\n");
        fake.write("sys/class/net/eth2/operstate", "up\n");
        fake.write("sys/class/net/eth2/device/vendor", "0x8086\n");
        let list = interfaces(&fake.paths());
        assert_eq!(
            list,
            [
                NetInterface {
                    name: "eth2".into(),
                    state: "up".into(),
                    physical: true
                },
                NetInterface {
                    name: "eth10".into(),
                    state: "down".into(),
                    physical: false
                },
            ]
        );
    }

    #[tokio::test]
    async fn sensors_read_their_files_and_rates_need_two_polls() {
        let fake = Fake::new();
        fake.write("proc/stat", "cpu  100 0 50 800 50 0 0 0 0 0\n");
        fake.write("proc/loadavg", "1.5 1.0 0.5 2/300 99\n");
        fake.write("sys/class/net/eth0/statistics/rx_bytes", "1000\n");
        fake.write("sys/class/net/eth0/statistics/tx_bytes", "0\n");

        let mut cpu = LocalSensor::new(LocalMetric::Cpu, fake.paths());
        let first = poll(&mut cpu, 0).await.unwrap();
        assert_eq!(values(&first), [("total", None), ("iowait", None)]);
        fake.write("proc/stat", "cpu  200 0 100 1600 100 0 0 0 0 0\n");
        let second = poll(&mut cpu, 60_000).await.unwrap();
        assert_eq!(
            values(&second),
            [("total", Some(15.0)), ("iowait", Some(5.0))]
        );

        let mut load = LocalSensor::new(LocalMetric::Load, fake.paths());
        assert_eq!(
            values(&poll(&mut load, 0).await.unwrap()),
            [
                ("load1", Some(1.5)),
                ("load5", Some(1.0)),
                ("load15", Some(0.5))
            ]
        );

        let eth0 = LocalMetric::Traffic {
            interface: "eth0".into(),
        };
        let mut net = LocalSensor::new(eth0, fake.paths());
        assert_eq!(
            values(&poll(&mut net, 0).await.unwrap()),
            [("in", None), ("out", None)]
        );
        fake.write("sys/class/net/eth0/statistics/rx_bytes", "11000\n");
        fake.write("sys/class/net/eth0/statistics/tx_bytes", "500\n");
        // 10 000 bytes in 10 s = 8 000 bit/s.
        assert_eq!(
            values(&poll(&mut net, 10_000).await.unwrap()),
            [("in", Some(8_000.0)), ("out", Some(400.0))]
        );

        let gone = LocalMetric::Traffic {
            interface: "eth9".into(),
        };
        let err = poll(&mut LocalSensor::new(gone, fake.paths()), 0).await;
        assert!(matches!(err, Err(SensorError::BadValue(m)) if m.contains("eth9")));
        let missing = poll(&mut LocalSensor::new(LocalMetric::Memory, fake.paths()), 0).await;
        assert!(matches!(missing, Err(SensorError::BadValue(_))));
    }

    #[tokio::test]
    async fn this_machine_can_be_read() {
        let paths = Paths::default();
        for metric in [LocalMetric::Load, LocalMetric::Memory, LocalMetric::Uptime] {
            let m = poll(&mut LocalSensor::new(metric.clone(), paths.clone()), 0)
                .await
                .unwrap();
            assert!(m.channels.iter().all(|c| c.value.is_some()), "{metric:?}");
        }
        assert!(filesystems(&paths).iter().any(|f| f.mount == "/"));
        assert!(!hostname(&paths).is_empty());
    }
}
