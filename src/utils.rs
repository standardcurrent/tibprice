use std::time::Duration;

/// Formats milliseconds into a human-readable duration string
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{}ms", ms);
    }

    // Round to the nearest second when displaying in seconds or larger units
    let seconds_float = ms as f64 / 1000.0;
    let seconds = seconds_float.round() as u64;

    if seconds < 60 {
        return format!("{}s", seconds);
    }

    let minutes = seconds / 60;
    let rem_seconds = seconds % 60;
    if minutes < 60 {
        if rem_seconds == 0 {
            return format!("{}m", minutes);
        }
        return format!("{}m {}s", minutes, rem_seconds);
    }

    let hours = minutes / 60;
    let rem_minutes = minutes % 60;
    if rem_minutes == 0 {
        return format!("{}h", hours);
    }
    return format!("{}h {}m", hours, rem_minutes);
}

/// Formats a std::time::Duration into a human-readable string
pub fn format_std_duration(duration: Duration) -> String {
    format_duration(duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(500 - 1), "499ms");
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(1000 - 1), "999ms");
        assert_eq!(format_duration(1000), "1s");
        assert_eq!(format_duration(5000 - 1), "5s");
        assert_eq!(format_duration(5000), "5s");
        assert_eq!(format_duration(60000 - 1), "1m");
        assert_eq!(format_duration(60000), "1m");
        assert_eq!(format_duration(90000 - 1), "1m 30s");
        assert_eq!(format_duration(90000), "1m 30s");
        assert_eq!(format_duration(3600000 - 1), "1h");
        assert_eq!(format_duration(3600000), "1h");
        assert_eq!(format_duration(3660000), "1h 1m");
        assert_eq!(format_duration(7200000), "2h");
        assert_eq!(format_duration(7260000), "2h 1m");
        assert_eq!(format_duration(9000000 - 1), "2h 30m");
        assert_eq!(format_duration(9000000), "2h 30m");
    }

    #[test]
    fn test_format_std_duration() {
        assert_eq!(format_std_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_std_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_std_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_std_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_std_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_std_duration(Duration::from_secs(3660)), "1h 1m");
    }
}
