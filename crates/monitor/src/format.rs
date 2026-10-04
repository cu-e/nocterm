//! Compact, human-readable quantities.

const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];

/// Bytes in binary units: `512B`, `3.1G`, `12G`.
pub fn bytes(value: u64) -> String {
    scaled(value as f64)
}

/// Bytes per second: `0B/s`, `1.2M/s`.
pub fn rate(bytes_per_second: f64) -> String {
    format!("{}/s", scaled(bytes_per_second.max(0.0)))
}

/// A percentage without a fraction: `7%`, `100%`.
pub fn percent(value: f32) -> String {
    format!("{:.0}%", value.clamp(0.0, 100.0))
}

/// Degrees Celsius: `47°C`.
pub fn celsius(value: f32) -> String {
    format!("{value:.0}°C")
}

/// A duration in its two largest units: `3d 4h`, `5h 12m`, `42s`.
pub fn duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

fn scaled(mut value: f64) -> String {
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // One decimal only while it carries information.
    if unit == 0 || value >= 10.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_are_compact() {
        assert_eq!(bytes(512), "512B");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024 + 100 * 1024 * 1024), "3.1G");
        assert_eq!(bytes(16 * 1024 * 1024 * 1024), "16G");
        assert_eq!(rate(1536.0), "1.5K/s");
        assert_eq!(rate(-1.0), "0B/s");
        assert_eq!(percent(7.4), "7%");
        assert_eq!(percent(140.0), "100%");
        assert_eq!(duration(3.0 * 86_400.0 + 4.0 * 3600.0 + 59.0), "3d 4h");
        assert_eq!(duration(5.0 * 3600.0 + 12.0 * 60.0), "5h 12m");
        assert_eq!(duration(42.0), "42s");
        assert_eq!(celsius(47.4), "47°C");
    }
}
