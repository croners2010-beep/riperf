use std::os::unix::io::AsRawFd;

// ─────────────────────────────────────────────────────────────
// Парсинг bandwidth с суффиксами KMG (1000-based, SI)
// ─────────────────────────────────────────────────────────────

/// Парсит строку вида "10M" или "10M/50K" (SI: K=1000, M=1000², G=1000³).
/// Используется для пропускной способности (биты/сек).
/// Формат "10M/50K" означает bandwidth=10M, burst=50K (burst игнорируется здесь).
pub fn parse_bandwidth(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() || s == "0" {
        return Ok(0);
    }

    // Берём только первую часть до '/'
    let s = s.split('/').next().unwrap_or(s);

    let b = s.as_bytes();
    let last = b[b.len() - 1];

    let (num_str, mult): (&str, u64) = match last {
        b'k' | b'K' => (&s[..s.len() - 1], 1_000),
        b'm' | b'M' => (&s[..s.len() - 1], 1_000_000),
        b'g' | b'G' => (&s[..s.len() - 1], 1_000_000_000),
        _ => (s, 1),
    };

    // Парсим как integer для точности
    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid bandwidth: {}", num_str))?;

    // Проверка переполнения
    n.checked_mul(mult)
        .ok_or_else(|| format!("bandwidth overflow: {} * {}", n, mult))
}

// ─────────────────────────────────────────────────────────────
// Socket options
// ─────────────────────────────────────────────────────────────

extern "C" {
    fn setsockopt(
        fd: i32,
        level: i32,
        optname: i32,
        optval: *const std::ffi::c_void,
        optlen: u32,
    ) -> i32;
}

// POSIX constants (Linux/macOS)
const SOL_SOCKET: i32 = 1;
const SO_SNDBUF: i32 = 7;
const SO_RCVBUF: i32 = 8;

// TCP level (Linux: IPPROTO_TCP=6, macOS: IPPROTO_TCP=6)
const IPPROTO_TCP: i32 = 6;
const TCP_MAXSEG: i32 = 2;

// IP level (IPPROTO_IP=0)
const IPPROTO_IP: i32 = 0;
const IP_TOS: i32 = 1;

/// Устанавливает размер буферов отправки и приёма сокета.
pub fn set_sock_buf<S: AsRawFd>(sock: &S, size: u32) {
    let fd = sock.as_raw_fd();
    let sz = size as i32;
    unsafe {
        let _ = setsockopt(
            fd,
            SOL_SOCKET,
            SO_SNDBUF,
            &sz as *const i32 as *const std::ffi::c_void,
            4,
        );
        let _ = setsockopt(
            fd,
            SOL_SOCKET,
            SO_RCVBUF,
            &sz as *const i32 as *const std::ffi::c_void,
            4,
        );
    }
}

/// Устанавливает TCP Maximum Segment Size (MSS).
pub fn set_tcp_mss<S: AsRawFd>(sock: &S, mss: u32) {
    let fd = sock.as_raw_fd();
    let mss_val = mss as i32;
    unsafe {
        let _ = setsockopt(
            fd,
            IPPROTO_TCP,
            TCP_MAXSEG,
            &mss_val as *const i32 as *const std::ffi::c_void,
            4,
        );
    }
}

/// Устанавливает IP Type of Service (ToS).
pub fn set_tos<S: AsRawFd>(sock: &S, tos: u32) {
    let fd = sock.as_raw_fd();
    let tos_val = tos as i32;
    unsafe {
        let _ = setsockopt(
            fd,
            IPPROTO_IP,
            IP_TOS,
            &tos_val as *const i32 as *const std::ffi::c_void,
            4,
        );
    }
}

// ─────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bandwidth_zero() {
        assert_eq!(parse_bandwidth("0").unwrap(), 0);
        assert_eq!(parse_bandwidth("").unwrap(), 0);
    }

    #[test]
    fn test_parse_bandwidth_plain() {
        assert_eq!(parse_bandwidth("1000000").unwrap(), 1_000_000);
    }

    #[test]
    fn test_parse_bandwidth_kilobits() {
        assert_eq!(parse_bandwidth("1K").unwrap(), 1_000);
        assert_eq!(parse_bandwidth("100k").unwrap(), 100_000);
    }

    #[test]
    fn test_parse_bandwidth_megabits() {
        assert_eq!(parse_bandwidth("1M").unwrap(), 1_000_000);
        assert_eq!(parse_bandwidth("10m").unwrap(), 10_000_000);
    }

    #[test]
    fn test_parse_bandwidth_gigabits() {
        assert_eq!(parse_bandwidth("1G").unwrap(), 1_000_000_000);
        assert_eq!(parse_bandwidth("2g").unwrap(), 2_000_000_000);
    }

    #[test]
    fn test_parse_bandwidth_with_burst() {
        // "10M/50K" → берём только "10M"
        assert_eq!(parse_bandwidth("10M/50K").unwrap(), 10_000_000);
        assert_eq!(parse_bandwidth("1G/100M").unwrap(), 1_000_000_000);
    }

    #[test]
    fn test_parse_bandwidth_invalid() {
        assert!(parse_bandwidth("abc").is_err());
        assert!(parse_bandwidth("10X").is_err());
    }

    #[test]
    fn test_parse_bandwidth_overflow() {
        assert!(parse_bandwidth("99999999999999999999G").is_err());
    }


}
