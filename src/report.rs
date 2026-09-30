
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::protocol::{StreamStats, TestResult};

// ─────────────────────────────────────────────────────────────
// Защита от NaN/Infinity
// ─────────────────────────────────────────────────────────────

/// Возвращает значение, если оно конечное, иначе 0.0
fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

// ─────────────────────────────────────────────────────────────
// Форматирование размеров
// ─────────────────────────────────────────────────────────────

/// Форматирует размер в байтах с SI-префиксами (1000-based).
pub fn format_transfer(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e12 {
        format!("{:.2} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.2} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.2} kB", b / 1e3)
    } else {
        format!("{} B", bytes)
    }
}

/// Форматирует битрейт в соответствии с указанным форматом вывода.
pub fn format_bitrate(bits_per_sec: f64, fmt: &OutputFormat) -> String {
    let rate = finite_or_zero(bits_per_sec);
    match fmt {
        OutputFormat::Kilobits => format!("{:.2} Kbit/s", rate / 1e3),
        OutputFormat::Megabits => format!("{:.2} Mbit/s", rate / 1e6),
        OutputFormat::Gigabits => format!("{:.2} Gbit/s", rate / 1e9),
        OutputFormat::Kilobytes => format!("{:.2} kB/s", rate / 8.0 / 1e3),
        OutputFormat::Megabytes => format!("{:.2} MB/s", rate / 8.0 / 1e6),
        OutputFormat::Gigabytes => format!("{:.2} GB/s", rate / 8.0 / 1e9),
    }
}

// ─────────────────────────────────────────────────────────────
// Вычисление битрейта
// ─────────────────────────────────────────────────────────────

fn bitrate_of(s: &StreamStats) -> f64 {
    if !s.duration_sec.is_finite() || s.duration_sec <= 0.0 {
        return 0.0;
    }
    let bitrate = (s.bytes as f64 * 8.0) / s.duration_sec;
    finite_or_zero(bitrate)
}

// ─────────────────────────────────────────────────────────────
// Текстовый вывод
// ─────────────────────────────────────────────────────────────

/// Рендерит текстовый отчёт в строку (для тестирования и логирования).
pub fn render_text(
    label: &str,
    result: &TestResult,
    fmt: &OutputFormat,
    title: Option<&str>,
) -> String {
    let mut output = String::new();
    let prefix = title.map(|t| format!("{} ", t)).unwrap_or_default();
    let role = if result.is_sender { "sender" } else { "receiver" };

    output.push_str(&format!("\n{}--- {} ---\n", prefix, label));
    if result.is_udp {
        output.push_str(&format!(
            "{}[ ID]   Interval          Transfer      Bitrate         Jitter     Lost/Total\n",
            prefix
        ));
    } else {
        output.push_str(&format!(
            "{}[ ID]   Interval          Transfer      Bitrate\n",
            prefix
        ));
    }

    for s in &result.streams {
        let duration = finite_or_zero(s.duration_sec);
        let interval = format!("0.00-{:.2} sec", duration);
        let id = format!("[{}]", s.stream_id);

        if result.is_udp {
            output.push_str(&format!(
                "{}{:<6} {:<17} {:<13} {:<15} {:<10.3} {}/{} {}\n",
                prefix, id, interval,
                format_transfer(s.bytes),
                format_bitrate(bitrate_of(s), fmt),
                finite_or_zero(s.jitter_ms),
                s.lost_packets, s.total_packets, role
            ));
        } else {
            output.push_str(&format!(
                "{}{:<6} {:<17} {:<13} {:<15} {}\n",
                prefix, id, interval,
                format_transfer(s.bytes),
                format_bitrate(bitrate_of(s), fmt), role
            ));
        }
    }

    // SUM row
    let sum = &result.sum;
    let duration = finite_or_zero(sum.duration_sec);
    let interval = format!("0.00-{:.2} sec", duration);

    if result.is_udp {
        output.push_str(&format!(
            "{}{:<6} {:<17} {:<13} {:<15} {:<10.3} {}/{} {}\n",
            prefix, "[SUM]", interval,
            format_transfer(sum.bytes),
            format_bitrate(bitrate_of(sum), fmt),
            finite_or_zero(sum.jitter_ms),
            sum.lost_packets, sum.total_packets, role
        ));
    } else {
        output.push_str(&format!(
            "{}{:<6} {:<17} {:<13} {:<15} {}\n",
            prefix, "[SUM]", interval,
            format_transfer(sum.bytes),
            format_bitrate(bitrate_of(sum), fmt), role
        ));
    }

    output
}

/// Печатает результат теста в текстовом формате.
pub fn print_result(
    label: &str,
    result: &TestResult,
    fmt: &OutputFormat,
    title: Option<&str>,
) {
    print!("{}", render_text(label, result, fmt, title));
}

// ─────────────────────────────────────────────────────────────
// JSON вывод (через serde_json)
// ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct JsonStream {
    id: usize,
    bytes: u64,
    packets: u64,
    duration_sec: f64,
    bitrate_bits_per_sec: f64,
    jitter_ms: f64,
    lost_packets: u64,
    total_packets: u64,
}

#[derive(Serialize)]
struct JsonResult<'a> {
    label: &'a str,
    role: &'a str,
    is_sender: bool,
    is_udp: bool,
    reverse: bool,
    streams: Vec<JsonStream>,
    sum: JsonStream,
}

fn stream_to_json(s: &StreamStats) -> JsonStream {
    JsonStream {
        id: s.stream_id,
        bytes: s.bytes,
        packets: s.packets,
        duration_sec: finite_or_zero(s.duration_sec),
        bitrate_bits_per_sec: finite_or_zero(bitrate_of(s)),
        jitter_ms: finite_or_zero(s.jitter_ms),
        lost_packets: s.lost_packets,
        total_packets: s.total_packets,
    }
}

/// Рендерит JSON-отчёт в строку (для тестирования и логирования).
pub fn render_json(label: &str, result: &TestResult) -> Result<String, serde_json::Error> {
    let role = if result.is_sender { "sender" } else { "receiver" };

    let output = JsonResult {
        label,
        role,
        is_sender: result.is_sender,
        is_udp: result.is_udp,
        reverse: result.reverse,
        streams: result.streams.iter().map(stream_to_json).collect(),
        sum: stream_to_json(&result.sum),
    };

    serde_json::to_string(&output)
}

/// Печатает результат теста в JSON-формате.
pub fn print_json(label: &str, result: &TestResult) {
    match render_json(label, result) {
        Ok(json) => println!("{}", json),
        Err(e) => eprintln!("failed to serialize result as JSON: {}", e),
    }
}

// ─────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_transfer_bytes() {
        assert_eq!(format_transfer(500), "500 B");
        assert_eq!(format_transfer(999), "999 B");
    }

    #[test]
    fn test_format_transfer_kilobytes() {
        assert_eq!(format_transfer(1000), "1.00 kB");
        assert_eq!(format_transfer(1500), "1.50 kB");
    }

    #[test]
    fn test_format_transfer_megabytes() {
        assert_eq!(format_transfer(1_000_000), "1.00 MB");
        assert_eq!(format_transfer(1_500_000), "1.50 MB");
    }

    #[test]
    fn test_format_transfer_gigabytes() {
        assert_eq!(format_transfer(1_000_000_000), "1.00 GB");
        assert_eq!(format_transfer(1_500_000_000), "1.50 GB");
    }

    #[test]
    fn test_format_bitrate_megabits() {
        let result = format_bitrate(1_000_000.0, &OutputFormat::Megabits);
        assert_eq!(result, "1.00 Mbit/s");
    }

    #[test]
    fn test_format_bitrate_nan() {
        let result = format_bitrate(f64::NAN, &OutputFormat::Megabits);
        assert_eq!(result, "0.00 Mbit/s");
    }

    #[test]
    fn test_format_bitrate_infinity() {
        let result = format_bitrate(f64::INFINITY, &OutputFormat::Megabits);
        assert_eq!(result, "0.00 Mbit/s");
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
    fn test_bitrate_of_negative_duration() {
        let stats = StreamStats {
            bytes: 1000,
            duration_sec: -1.0,
            ..Default::default()
        };
        assert_eq!(bitrate_of(&stats), 0.0);
    }

    #[test]
    fn test_bitrate_of_nan_duration() {
        let stats = StreamStats {
            bytes: 1000,
            duration_sec: f64::NAN,
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

    #[test]
    fn test_json_is_valid() {
        let result = TestResult {
            streams: vec![StreamStats {
                stream_id: 0,
                bytes: 1_000_000,
                packets: 100,
                duration_sec: 10.0,
                jitter_ms: 0.5,
                lost_packets: 2,
                total_packets: 102,
            }],
            sum: StreamStats {
                stream_id: 0,
                bytes: 1_000_000,
                packets: 100,
                duration_sec: 10.0,
                jitter_ms: 0.5,
                lost_packets: 2,
                total_packets: 102,
            },
            is_sender: true,
            is_udp: false,
            reverse: false,
        };

        let json = render_json("test", &result).unwrap();
        // Проверяем, что JSON валиден
        let _: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_json_escapes_label() {
        let result = TestResult::default();
        // label содержит кавычки, backslash и newline
        let json = render_json("test\"abc\\def\nghi", &result).unwrap();
        // Проверяем, что JSON валиден
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["label"], "test\"abc\\def\nghi");
    }

    #[test]
    fn test_json_contains_bitrate() {
        let result = TestResult {
            streams: vec![StreamStats {
                stream_id: 0,
                bytes: 1_000_000,
                duration_sec: 1.0,
                ..Default::default()
            }],
            sum: StreamStats::default(),
            is_sender: true,
            is_udp: false,
            reverse: false,
        };

        let json = render_json("test", &result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Проверяем наличие bitrate_bits_per_sec
        assert!(parsed["streams"][0]["bitrate_bits_per_sec"].is_number());
        assert_eq!(parsed["streams"][0]["bitrate_bits_per_sec"], 8_000_000.0);
    }

    #[test]
    fn test_json_separates_label_and_role() {
        let result = TestResult {
            streams: vec![],
            sum: StreamStats::default(),
            is_sender: true,
            is_udp: false,
            reverse: false,
        };

        let json = render_json("my_label", &result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["label"], "my_label");
        assert_eq!(parsed["role"], "sender");
    }

    #[test]
    fn test_json_handles_nan() {
        let result = TestResult {
            streams: vec![StreamStats {
                stream_id: 0,
                bytes: 1000,
                duration_sec: f64::NAN,
                jitter_ms: f64::INFINITY,
                ..Default::default()
            }],
            sum: StreamStats::default(),
            is_sender: true,
            is_udp: false,
            reverse: false,
        };

        let json = render_json("test", &result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // NaN и Infinity должны быть заменены на 0.0
        assert_eq!(parsed["streams"][0]["duration_sec"], 0.0);
        assert_eq!(parsed["streams"][0]["jitter_ms"], 0.0);
    }
}
