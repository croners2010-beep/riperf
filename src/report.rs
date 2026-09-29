
use crate::cli::OutputFormat;
use crate::protocol::{StreamStats, TestResult};

// ─────────────────────────────────────────────────────────────
// Форматирование размеров
// ─────────────────────────────────────────────────────────────

/// Форматирует размер в байтах с SI-префиксами (1000-based).
pub fn format_transfer(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e12 {
        format!("{:.2} TBytes", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.2} GBytes", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.2} MBytes", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.2} KBytes", b / 1e3)
    } else {
        format!("{} Bytes", bytes)
    }
}

/// Форматирует битрейт в соответствии с указанным форматом вывода.
pub fn format_bitrate(bits_per_sec: f64, fmt: &OutputFormat) -> String {
    match fmt {
        OutputFormat::Kilobits => format!("{:.2} Kbits/sec", bits_per_sec / 1e3),
        OutputFormat::Megabits => format!("{:.2} Mbits/sec", bits_per_sec / 1e6),
        OutputFormat::Gigabits => format!("{:.2} Gbits/sec", bits_per_sec / 1e9),
        OutputFormat::Kilobytes => format!("{:.2} KBytes/sec", bits_per_sec / 8.0 / 1e3),
        OutputFormat::Megabytes => format!("{:.2} MBytes/sec", bits_per_sec / 8.0 / 1e6),
        OutputFormat::Gigabytes => format!("{:.2} GBytes/sec", bits_per_sec / 8.0 / 1e9),
    }
}

// ─────────────────────────────────────────────────────────────
// Вычисление битрейта
// ─────────────────────────────────────────────────────────────

fn bitrate_of(s: &StreamStats) -> f64 {
    if s.duration_sec > 0.0 {
        (s.bytes as f64 * 8.0) / s.duration_sec
    } else {
        0.0
    }
}

// ─────────────────────────────────────────────────────────────
// Вывод результатов
// ─────────────────────────────────────────────────────────────

fn print_row(
    prefix: &str,
    s: &StreamStats,
    fmt: &OutputFormat,
    is_udp: bool,
    role: &str,
    is_sum: bool,
) {
    let interval = format!("0.00-{:.2} sec", s.duration_sec);
    let id = if is_sum {
        "[SUM]".to_string()
    } else {
        format!("[{}]", s.stream_id)
    };

    if is_udp {
        println!(
            "{}{:<6} {:<17} {:<13} {:<15} {:<10.3} {}/{} {}",
            prefix,
            id,
            interval,
            format_transfer(s.bytes),
            format_bitrate(bitrate_of(s), fmt),
            s.jitter_ms,
            s.lost_packets,
            s.total_packets,
            role
        );
    } else {
        println!(
            "{}{:<6} {:<17} {:<13} {:<15} {}",
            prefix,
            id,
            interval,
            format_transfer(s.bytes),
            format_bitrate(bitrate_of(s), fmt),
            role
        );
    }
}

/// Печатает результат теста в текстовом формате.
pub fn print_result(
    label: &str,
    result: &TestResult,
    fmt: &OutputFormat,
    title: Option<&str>,
) {
    let prefix = title.map(|t| format!("{} ", t)).unwrap_or_default();
    let role = if result.is_sender { "sender" } else { "receiver" };

    println!();
    println!("{}--- {} ---", prefix, label);
    if result.is_udp {
        println!(
            "{}[ ID]   Interval          Transfer      Bitrate         Jitter     Lost/Total",
            prefix
        );
    } else {
        println!("{}[ ID]   Interval          Transfer      Bitrate", prefix);
    }
    for s in &result.streams {
        print_row(&prefix, s, fmt, result.is_udp, role, false);
    }
    print_row(&prefix, &result.sum, fmt, result.is_udp, role, true);
}

/// Печатает результат теста в JSON-формате.
pub fn print_json(label: &str, result: &TestResult) {
    let streams: Vec<String> = result
        .streams
        .iter()
        .map(|s| {
            format!(
                r#"{{"id":{},"bytes":{},"packets":{},"duration":{:.6},"jitter_ms":{:.6},"lost_packets":{},"total_packets":{}}}"#,
                s.stream_id, s.bytes, s.packets, s.duration_sec, s.jitter_ms,
                s.lost_packets, s.total_packets
            )
        })
        .collect();

    let sum = format!(
        r#"{{"bytes":{},"packets":{},"duration":{:.6},"jitter_ms":{:.6},"lost_packets":{},"total_packets":{}}}"#,
        result.sum.bytes,
        result.sum.packets,
        result.sum.duration_sec,
        result.sum.jitter_ms,
        result.sum.lost_packets,
        result.sum.total_packets
    );

    let obj = format!(
        r#"{{"role":"{}","is_sender":{},"is_udp":{},"reverse":{},"streams":[{}],"sum":{}}}"#,
        label,
        result.is_sender,
        result.is_udp,
        result.reverse,
        streams.join(","),
        sum
    );

    println!("{}", obj);
}

// ─────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_transfer_bytes() {
        assert_eq!(format_transfer(500), "500 Bytes");
    }

    #[test]
    fn test_format_transfer_kbytes() {
        assert_eq!(format_transfer(1500), "1.50 KBytes");
    }

    #[test]
    fn test_format_transfer_mbytes() {
        assert_eq!(format_transfer(1_500_000), "1.50 MBytes");
    }

    #[test]
    fn test_format_transfer_gbytes() {
        assert_eq!(format_transfer(1_500_000_000), "1.50 GBytes");
    }

    #[test]
    fn test_format_transfer_tbytes() {
        assert_eq!(format_transfer(1_500_000_000_000), "1.50 TBytes");
    }

    #[test]
    fn test_format_bitrate_kilobits() {
        let result = format_bitrate(1_000_000.0, &OutputFormat::Kilobits);
        assert_eq!(result, "1000.00 Kbits/sec");
    }

    #[test]
    fn test_format_bitrate_megabits() {
        let result = format_bitrate(1_000_000.0, &OutputFormat::Megabits);
        assert_eq!(result, "1.00 Mbits/sec");
    }

    #[test]
    fn test_format_bitrate_gigabits() {
        let result = format_bitrate(1_000_000_000.0, &OutputFormat::Gigabits);
        assert_eq!(result, "1.00 Gbits/sec");
    }

    #[test]
    fn test_format_bitrate_kilobytes() {
        // 8_000_000 bits/sec = 1_000_000 bytes/sec = 1000 KBytes/sec
        let result = format_bitrate(8_000_000.0, &OutputFormat::Kilobytes);
        assert_eq!(result, "1000.00 KBytes/sec");
    }

    #[test]
    fn test_format_bitrate_megabytes() {
        // 8_000_000 bits/sec = 1_000_000 bytes/sec = 1 MBytes/sec
        let result = format_bitrate(8_000_000.0, &OutputFormat::Megabytes);
        assert_eq!(result, "1.00 MBytes/sec");
    }

    #[test]
    fn test_format_bitrate_gigabytes() {
        // 8_000_000_000 bits/sec = 1_000_000_000 bytes/sec = 1 GBytes/sec
        let result = format_bitrate(8_000_000_000.0, &OutputFormat::Gigabytes);
        assert_eq!(result, "1.00 GBytes/sec");
    }

    #[test]
    fn test_bitrate_of_zero_duration() {
        let stats = StreamStats {
            bytes: 1000,
            duration_sec: 0.0,
            ..Default::default()
        };
        assert_eq!(bitrate_of(&stats), 0.0);
    }

    #[test]
    fn test_bitrate_of_normal() {
        let stats = StreamStats {
            bytes: 1_000_000, // 1 MB = 8 Mbits
            duration_sec: 1.0,
            ..Default::default()
        };
        assert_eq!(bitrate_of(&stats), 8_000_000.0);
    }
}
