
use std::fmt;

// ─────────────────────────────────────────────────────────────
// Типы ошибок
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingValue { option: String },
    InvalidValue {
        option: String,
        value: String,
        reason: &'static str,
    },
    UnknownOption(String),
    Validation(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue { option } => {
                write!(f, "option '{}' requires a value", option)
            }
            Self::InvalidValue {
                option,
                value,
                reason,
            } => {
                write!(
                    f,
                    "invalid value '{}' for {}: {}",
                    value, option, reason
                )
            }
            Self::UnknownOption(opt) => write!(f, "unknown option: {}", opt),
            Self::Validation(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

// ─────────────────────────────────────────────────────────────
// Формат вывода
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Kilobits,
    Megabits,
    Gigabits,
    Kilobytes,
    Megabytes,
    Gigabytes,
}

impl OutputFormat {
    fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "k" => Some(Self::Kilobits),
            "m" => Some(Self::Megabits),
            "g" => Some(Self::Gigabits),
            "K" => Some(Self::Kilobytes),
            "M" => Some(Self::Megabytes),
            "G" => Some(Self::Gigabytes),
            _ => None,
        }
    }
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Megabits
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = match self {
            Self::Kilobits => 'k',
            Self::Megabits => 'm',
            Self::Gigabits => 'g',
            Self::Kilobytes => 'K',
            Self::Megabytes => 'M',
            Self::Gigabytes => 'G',
        };
        write!(f, "{}", c)
    }
}

// ─────────────────────────────────────────────────────────────
// Структура аргументов
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Args {
    // Общие
    pub help: bool,
    pub version: bool,
    pub port: u16,
    pub format: OutputFormat,
    pub interval: f64,
    pub file: Option<String>,
    pub bind: Option<String>,
    pub verbose: bool,
    pub json: bool,
    pub logfile: Option<String>,
    pub debug: bool,

    // Серверные
    pub server: bool,
    pub daemon: bool,
    pub pidfile: Option<String>,
    pub one_off: bool,

    // Клиентские
    pub client: Option<String>,
    pub udp: bool,
    pub bandwidth: String,
    pub time: u64,
    pub bytes: Option<u64>,
    pub blockcount: Option<u64>,
    pub buffer_len: Option<u64>,
    pub client_port: Option<u16>,
    pub parallel: usize,
    pub reverse: bool,
    pub window: Option<u64>,
    pub mss: Option<u32>,
    pub nodelay: bool,
    pub ipv4: bool,
    pub ipv6: bool,
    pub tos: Option<u32>,
    pub zerocopy: bool,
    pub omit: u64,
    pub title: Option<String>,
    pub get_server_output: bool,
    pub udp_counters_64bit: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            help: false,
            version: false,
            server: false,
            client: None,
            port: 5201,
            format: OutputFormat::default(),
            interval: 1.0,
            file: None,
            bind: None,
            verbose: false,
            json: false,
            logfile: None,
            debug: false,
            daemon: false,
            pidfile: None,
            one_off: false,
            udp: false,
            bandwidth: "0".into(),
            time: 10,
            bytes: None,
            blockcount: None,
            buffer_len: None,
            client_port: None,
            parallel: 1,
            reverse: false,
            window: None,
            mss: None,
            nodelay: false,
            ipv4: false,
            ipv6: false,
            tos: None,
            zerocopy: false,
            omit: 0,
            title: None,
            get_server_output: false,
            udp_counters_64bit: false,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Парсинг суффиксов KMG
// ─────────────────────────────────────────────────────────────

/// Парсит строку вида "123", "10K", "5M", "1G" (регистрозависимо: K/M/G = 1024, k/m/g тоже 1024).
/// Формат: число (опционально) + суффикс (опционально).
/// Примеры: "123", "10K", "5M", "1G", "10k", "5m", "1g"
pub fn parse_with_suffix(s: &str) -> Result<u64, ()> {
    let s = s.trim();
    if s.is_empty() {
        return Err(());
    }

    // Найти первый нецифровой символ — это граница между числом и суффиксом
    let split_pos = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());

    let (num_part, suffix) = s.split_at(split_pos);

    // Число должно быть непустым
    if num_part.is_empty() {
        return Err(());
    }

    let base: u64 = num_part.parse().map_err(|_| ())?;

    let multiplier: u64 = match suffix {
        "" => 1,
        "k" | "K" => 1024,
        "m" | "M" => 1024 * 1024,
        "g" | "G" => 1024 * 1024 * 1024,
        _ => return Err(()),
    };

    base.checked_mul(multiplier).ok_or(())
}

/// Парсит bandwidth вида "10M" или "10M/50K" (с burst).
/// Использует SI-суффиксы (1000-based): K=1000, M=1000², G=1000³
/// Возвращает (bandwidth_bits, burst_opt).
pub fn parse_bandwidth(s: &str) -> Result<(u64, Option<u64>), ()> {
    let parse_si_suffix = |s: &str| -> Result<u64, ()> {
        let s = s.trim();
        if s.is_empty() {
            return Err(());
        }

        let split_pos = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (num_part, suffix) = s.split_at(split_pos);

        if num_part.is_empty() {
            return Err(());
        }

        let base: u64 = num_part.parse().map_err(|_| ())?;

        // SI-суффиксы (1000-based) для bandwidth
        let multiplier: u64 = match suffix {
            "" => 1,
            "k" | "K" => 1_000,
            "m" | "M" => 1_000_000,
            "g" | "G" => 1_000_000_000,
            _ => return Err(()),
        };

        base.checked_mul(multiplier).ok_or(())
    };

    if let Some((bw_part, burst_part)) = s.split_once('/') {
        let bw = parse_si_suffix(bw_part)?;
        let burst = parse_si_suffix(burst_part)?;
        Ok((bw, Some(burst)))
    } else {
        let bw = parse_si_suffix(s)?;
        Ok((bw, None))
    }
}

// ─────────────────────────────────────────────────────────────
// Предварительная обработка аргументов
// ─────────────────────────────────────────────────────────────

/// Определяет, требует ли короткий флаг значения.
fn short_flag_requires_value(flag: &str) -> bool {
    matches!(
        flag,
        "-c" | "-p"
            | "-f"
            | "-i"
            | "-F"
            | "-B"
            | "-I"
            | "-b"
            | "-t"
            | "-n"
            | "-k"
            | "-l"
            | "-P"
            | "-w"
            | "-M"
            | "-S"
            | "-O"
            | "-T"
    )
}

/// Развёртывает `--key=value` в два отдельных аргумента: `--key` и `value`.
fn expand_equals(args: Vec<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(args.len());
    for arg in args {
        if arg.starts_with("--") {
            if let Some((key, val)) = arg.split_once('=') {
                result.push(key.to_string());
                result.push(val.to_string());
            } else {
                result.push(arg);
            }
        } else {
            result.push(arg);
        }
    }
    result
}

/// Развёртывает группировку коротких флагов: `-suV` → `-s`, `-u`, `-V`.
/// Если флаг требует значения, остаток строки становится значением: `-clocalhost` → `-c`, `localhost`.
/// ВАЖНО: если остаток начинается с `-`, он НЕ разворачивается (защита от `-T -foo`).
fn expand_short_flags(args: Vec<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(args.len());
    let mut skip_next = false;

    for arg in args.iter() {
        if skip_next {
            skip_next = false;
            continue;
        }

        if arg.starts_with('-')
            && !arg.starts_with("--")
            && arg.len() > 2
            && arg.as_bytes()[1] != b'-'
        {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let flag = format!("-{}", chars[i]);
                result.push(flag.clone());
                if short_flag_requires_value(&flag) {
                    // Остаток строки — значение
                    if i + 1 < chars.len() {
                        let value: String = chars[i + 1..].iter().collect();
                        // ВАЖНО: если значение начинается с '-', не разворачиваем его
                        result.push(value);
                    }
                    // Если i+1 == chars.len(), значение возьмётся из следующего аргумента
                    break;
                }
                i += 1;
            }
        } else {
            result.push(arg.clone());
        }
    }
    result
}

// ─────────────────────────────────────────────────────────────
// Парсер
// ─────────────────────────────────────────────────────────────

#[must_use = "parse result must be handled"]
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, ParseError> {
    // Предварительная обработка: --key=value и группировка коротких флагов
    let args: Vec<String> = expand_equals(expand_short_flags(args.into_iter().collect()));
    let mut it = args.into_iter();

    // Макрос: получить следующее значение или ошибку
    macro_rules! get_val {
        ($name:expr) => {
            it.next().ok_or_else(|| ParseError::MissingValue {
                option: $name.to_string(),
            })?
        };
    }

    // Макрос: получить значение и распарсить в указанный тип
    macro_rules! parse_val {
        ($name:expr, $type:ty) => {{
            let raw = get_val!($name);
            raw.parse::<$type>().map_err(|_| ParseError::InvalidValue {
                option: $name.to_string(),
                value: raw,
                reason: concat!("expected ", stringify!($type)),
            })?
        }};
    }

    // Макрос: получить значение с суффиксами KMG
    macro_rules! parse_suffix_val {
        ($name:expr) => {{
            let raw = get_val!($name);
            parse_with_suffix(&raw).map_err(|_| ParseError::InvalidValue {
                option: $name.to_string(),
                value: raw.clone(),
                reason: "expected number with optional K/M/G suffix",
            })?
        }};
    }

    let mut a = Args::default();

    while let Some(arg) = it.next() {
        match arg.as_str() {
            // ── Общие ──
            "-h" | "--help" => a.help = true,
            "-v" | "--version" => a.version = true,
            "-p" | "--port" => a.port = parse_val!(arg, u16),
            "-f" | "--format" => {
                let raw = get_val!(arg);
                a.format = OutputFormat::from_str_opt(&raw).ok_or_else(|| {
                    ParseError::InvalidValue {
                        option: arg.clone(),
                        value: raw,
                        reason: "expected one of: k, m, g, K, M, G",
                    }
                })?;
            }
            "-i" | "--interval" => a.interval = parse_val!(arg, f64),
            "-F" | "--file" => a.file = Some(get_val!(arg)),
            "-B" | "--bind" => a.bind = Some(get_val!(arg)),
            "-V" | "--verbose" => a.verbose = true,
            "-J" | "--json" => a.json = true,
            "--logfile" => a.logfile = Some(get_val!(arg)),
            "-d" | "--debug" => a.debug = true,

            // ── Серверные ──
            "-s" | "--server" => a.server = true,
            "-D" | "--daemon" => a.daemon = true,
            "-I" | "--pidfile" => a.pidfile = Some(get_val!(arg)),
            "-1" | "--one-off" => a.one_off = true,

            // ── Клиентские ──
            "-c" | "--client" => a.client = Some(get_val!(arg)),
            "-u" | "--udp" => a.udp = true,
            "-b" | "--bandwidth" => a.bandwidth = get_val!(arg),
            "-t" | "--time" => a.time = parse_val!(arg, u64),
            "-n" | "--bytes" => a.bytes = Some(parse_suffix_val!(arg)),
            "-k" | "--blockcount" => a.blockcount = Some(parse_suffix_val!(arg)),
            "-l" | "--len" => a.buffer_len = Some(parse_suffix_val!(arg)),
            "--cport" => a.client_port = Some(parse_val!(arg, u16)),
            "-P" | "--parallel" => {
                a.parallel = parse_val!(arg, usize);
            }
            "-R" | "--reverse" => a.reverse = true,
            "-w" | "--window" => a.window = Some(parse_suffix_val!(arg)),
            "-M" | "--set-mss" => a.mss = Some(parse_val!(arg, u32)),
            "-N" | "--no-delay" => a.nodelay = true,
            "-4" | "--version4" => a.ipv4 = true,
            "-6" | "--version6" => a.ipv6 = true,
            "-S" | "--tos" => a.tos = Some(parse_val!(arg, u32)),
            "-Z" | "--zerocopy" => a.zerocopy = true,
            "-O" | "--omit" => a.omit = parse_val!(arg, u64),
            "-T" | "--title" => a.title = Some(get_val!(arg)),
            "--get-server-output" => a.get_server_output = true,
            "--udp-counters-64bit" => a.udp_counters_64bit = true,

            _ => return Err(ParseError::UnknownOption(arg)),
        }
    }

    validate(&a)?;
    Ok(a)
}

// ─────────────────────────────────────────────────────────────
// Валидация
// ─────────────────────────────────────────────────────────────

fn validate(a: &Args) -> Result<(), ParseError> {
    // Режим: server XOR client (если не help/version)
    if !a.help && !a.version {
        if a.server && a.client.is_some() {
            return Err(ParseError::Validation(
                "cannot run as both server (-s) and client (-c)".into(),
            ));
        }
        if !a.server && a.client.is_none() {
            return Err(ParseError::Validation(
                "must specify either server (-s) or client (-c)".into(),
            ));
        }
    }

    // IPv4 XOR IPv6
    if a.ipv4 && a.ipv6 {
        return Err(ParseError::Validation(
            "cannot use both IPv4 (-4) and IPv6 (-6)".into(),
        ));
    }

    // Взаимоисключающие лимиты передачи
    let limits = [a.bytes.is_some(), a.blockcount.is_some()];
    if limits.iter().filter(|&&v| v).count() > 1 {
        return Err(ParseError::Validation(
            "options --bytes (-n) and --blockcount (-k) are mutually exclusive".into(),
        ));
    }

    // parallel > 0
    if a.parallel == 0 {
        return Err(ParseError::Validation(
            "--parallel (-P) must be at least 1".into(),
        ));
    }

    // interval > 0
    if a.interval <= 0.0 {
        return Err(ParseError::Validation(
            "--interval (-i) must be positive".into(),
        ));
    }

    // port != 0
    if a.port == 0 {
        return Err(ParseError::Validation(
            "--port (-p) must not be 0".into(),
        ));
    }

    // daemon/one_off/pidfile — только для сервера
    if a.client.is_some() {
        if a.daemon {
            return Err(ParseError::Validation(
                "--daemon (-D) is only valid in server mode".into(),
            ));
        }
        if a.pidfile.is_some() {
            return Err(ParseError::Validation(
                "--pidfile (-I) is only valid in server mode".into(),
            ));
        }
    }

    // Серверные опции не должны использоваться в клиентском режиме
    if a.server {
        let client_only = [
            (a.udp, "--udp (-u)"),
            (a.reverse, "--reverse (-R)"),
            (a.nodelay, "--no-delay (-N)"),
            (a.zerocopy, "--zerocopy (-Z)"),
        ];
        for (set, name) in client_only {
            if set {
                return Err(ParseError::Validation(format!(
                    "{} is only valid in client mode",
                    name
                )));
            }
        }
        if a.bandwidth != "0" {
            return Err(ParseError::Validation(
                "--bandwidth (-b) is only valid in client mode".into(),
            ));
        }
    }

    // Валидация bandwidth
    if a.bandwidth != "0" {
        parse_bandwidth(&a.bandwidth).map_err(|_| ParseError::InvalidValue {
            option: "--bandwidth".into(),
            value: a.bandwidth.clone(),
            reason: "expected number with optional K/M/G suffix, optionally /burst",
        })?;
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Вывод справки
// ─────────────────────────────────────────────────────────────

/// Печатает справку в stdout (для --help).
pub fn print_help() {
    println!(
        r#"Usage: riperf [-s|-c host] [options]
       riperf [-h|--help] [-v|--version]

Server or Client:
  -p, --port      #         server port to listen on/connect to
  -f, --format    [kmgKMG]  format to report
  -i, --interval  #         seconds between periodic bandwidth reports
  -F, --file name           xmit/recv the specified file
  -B, --bind      <host>    bind to a specific interface
  -V, --verbose             more detailed output
  -J, --json                output in JSON format
  --logfile f               send output to a log file
  -d, --debug               emit debugging output
  -v, --version             show version information and quit
  -h, --help                show this message and quit
Server specific:
  -s, --server              run in server mode
  -D, --daemon              run the server as a daemon
  -I, --pidfile file        write PID file
  -1, --one-off             handle one client connection then exit
Client specific:
  -c, --client    <host>    run in client mode, connecting to <host>
  -u, --udp                 use UDP rather than TCP
  -b, --bandwidth #[KMG][/#] target bandwidth in bits/sec
  -t, --time      #         time in seconds to transmit for (default 10)
  -n, --bytes     #[KMG]    number of bytes to transmit
  -k, --blockcount #[KMG]   number of blocks (packets) to transmit
  -l, --len       #[KMG]    length of buffer
  --cport         <port>    bind to a specific client port
  -P, --parallel  #         number of parallel client streams
  -R, --reverse             run in reverse mode
  -w, --window    #[KMG]    set window size / socket buffer size
  -M, --set-mss   #         set TCP maximum segment size
  -N, --no-delay            set TCP no delay
  -4, --version4            only use IPv4
  -6, --version6            only use IPv6
  -S, --tos N               set the IP 'type of service'
  -Z, --zerocopy            use a 'zero copy' method of sending data
  -O, --omit N              omit the first n seconds
  -T, --title str           prefix every output line with this string
  --get-server-output       get results from server
  --udp-counters-64bit      use 64-bit counters in UDP test packets
"#
    );
}



// ─────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Хелпер: парсинг из строки-команды.
    fn parse_str(s: &str) -> Result<Args, ParseError> {
        parse(s.split_whitespace().map(String::from))
    }



    // ── Базовый парсинг ──

    #[test]
    fn test_server_defaults() {
        let a = parse_str("-s").unwrap();
        assert!(a.server);
        assert_eq!(a.port, 5201);
        assert_eq!(a.format, OutputFormat::Megabits);
        assert_eq!(a.interval, 1.0);
        assert_eq!(a.time, 10);
        assert_eq!(a.parallel, 1);
        assert_eq!(a.bandwidth, "0");
    }

    #[test]
    fn test_client_basic() {
        let a = parse_str("-c 192.168.1.1").unwrap();
        assert_eq!(a.client.as_deref(), Some("192.168.1.1"));
        assert!(!a.server);
    }

    #[test]
    fn test_port_override() {
        let a = parse_str("-s -p 8080").unwrap();
        assert_eq!(a.port, 8080);
    }

    #[test]
    fn test_format_parsing() {
        let a = parse_str("-s -f K").unwrap();
        assert_eq!(a.format, OutputFormat::Kilobytes);

        let a = parse_str("-s -f g").unwrap();
        assert_eq!(a.format, OutputFormat::Gigabits);
    }

    #[test]
    fn test_invalid_format() {
        let err = parse_str("-s -f X").unwrap_err();
        assert!(matches!(err, ParseError::InvalidValue { .. }));
    }

    // ── --key=value ──

    #[test]
    fn test_equals_syntax() {
        // Строчная 'g' = Gigabits, заглавная 'G' = Gigabytes
        let a = parse_str("-s --port=9090 --format=g").unwrap();
        assert_eq!(a.port, 9090);
        assert_eq!(a.format, OutputFormat::Gigabits);
    }

    #[test]
    fn test_equals_client() {
        let a = parse_str("--client=10.0.0.1 --port=5000").unwrap();
        assert_eq!(a.client.as_deref(), Some("10.0.0.1"));
        assert_eq!(a.port, 5000);
    }

    // ── Группировка коротких флагов ──

    #[test]
    fn test_short_flag_grouping_client() {
        // -c требует значения → "uVJ" станет значением client
        // udp, verbose, json НЕ устанавливаются, т.к. uVJ — это значение для -c
        let a = parse_str("-cuVJ").unwrap();
        assert_eq!(a.client.as_deref(), Some("uVJ"));
        assert!(!a.udp);
        assert!(!a.verbose);
        assert!(!a.json);
    }

    #[test]
    fn test_short_flag_grouping_separate() {
        // -c host -uVJ → client=host, udp, verbose, json
        let a = parse_str("-c host -uVJ").unwrap();
        assert_eq!(a.client.as_deref(), Some("host"));
        assert!(a.udp);
        assert!(a.verbose);
        assert!(a.json);
    }

    #[test]
    fn test_short_flag_grouping_with_client() {
        // -c требует значения → "uVJ" станет значением client
        let a = parse_str("-cuVJ").unwrap();
        assert_eq!(a.client.as_deref(), Some("uVJ"));
    }

    #[test]
    fn test_short_grouping_bool_flags() {
        // -sVJd → server, verbose, json, debug
        let a = parse_str("-sVJd").unwrap();
        assert!(a.server);
        assert!(a.verbose);
        assert!(a.json);
        assert!(a.debug);
    }

    #[test]
    fn test_short_grouping_value_attached() {
        // -p9090 → port = 9090
        let a = parse_str("-sp9090").unwrap();
        assert!(a.server);
        assert_eq!(a.port, 9090);
    }

    #[test]
    fn test_short_grouping_mixed() {
        // -sVp9090 → server, verbose, port=9090
        let a = parse_str("-sVp9090").unwrap();
        assert!(a.server);
        assert!(a.verbose);
        assert_eq!(a.port, 9090);
    }

    // ── Суффиксы KMG ──

    #[test]
    fn test_suffix_parsing() {
        assert_eq!(parse_with_suffix("1024").unwrap(), 1024);
        assert_eq!(parse_with_suffix("10K").unwrap(), 10 * 1024);
        assert_eq!(parse_with_suffix("5M").unwrap(), 5 * 1024 * 1024);
        assert_eq!(parse_with_suffix("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_with_suffix("10k").unwrap(), 10 * 1024);
    }

    #[test]
    fn test_suffix_invalid() {
        assert!(parse_with_suffix("").is_err());
        assert!(parse_with_suffix("abc").is_err());
        assert!(parse_with_suffix("10X").is_err());
        // Некорректные строки с промежуточными символами
        assert!(parse_with_suffix("12abc3").is_err());
        assert!(parse_with_suffix("1K2").is_err());
        assert!(parse_with_suffix("123Kxyz3").is_err());
        assert!(parse_with_suffix("K10").is_err());
        assert!(parse_with_suffix("1K2M").is_err());
    }

    #[test]
    fn test_bytes_with_suffix() {
        let a = parse_str("-c host -n 10M").unwrap();
        assert_eq!(a.bytes, Some(10 * 1024 * 1024));
    }

    #[test]
    fn test_blockcount_with_suffix() {
        let a = parse_str("-c host -k 5K").unwrap();
        assert_eq!(a.blockcount, Some(5 * 1024));
    }

    #[test]
    fn test_buffer_len_with_suffix() {
        let a = parse_str("-c host -l 8K").unwrap();
        assert_eq!(a.buffer_len, Some(8 * 1024));
    }

    #[test]
    fn test_window_with_suffix() {
        let a = parse_str("-c host -w 64K").unwrap();
        assert_eq!(a.window, Some(64 * 1024));
    }

    // ── Bandwidth ──

    #[test]
    fn test_bandwidth_simple() {
        let (bw, _burst) = parse_bandwidth("10M").unwrap();
        assert_eq!(bw, 10_000_000); // SI: 10 * 1000 * 1000
    }

    #[test]
    fn test_bandwidth_with_burst() {
        let (bw, burst) = parse_bandwidth("10M/50K").unwrap();
        assert_eq!(bw, 10_000_000); // SI: 10 * 1000 * 1000
        assert_eq!(burst, Some(50_000)); // SI: 50 * 1000
    }

    #[test]
    fn test_bandwidth_default() {
        let a = parse_str("-s").unwrap();
        assert_eq!(a.bandwidth, "0");
    }

    // ── Валидация ──

    #[test]
    fn test_server_and_client_conflict() {
        let err = parse_str("-s -c localhost").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
        assert!(err.to_string().contains("both server"));
    }

    #[test]
    fn test_no_mode_specified() {
        let err = parse_str("-p 5201").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
        assert!(err.to_string().contains("must specify"));
    }

    #[test]
    fn test_ipv4_ipv6_conflict() {
        let err = parse_str("-c host -4 -6").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_bytes_and_blockcount_conflict() {
        let err = parse_str("-c host -n 100 -k 50").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_parallel_zero() {
        let err = parse_str("-c host -P 0").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_daemon_in_client_mode() {
        let err = parse_str("-c host -D").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_pidfile_in_client_mode() {
        let err = parse_str("-c host -I /tmp/test.pid").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_client_flag_in_server_mode() {
        let err = parse_str("-s -u").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_bandwidth_in_server_mode() {
        let err = parse_str("-s -b 10M").unwrap_err();
        assert!(matches!(err, ParseError::Validation(_)));
    }

    #[test]
    fn test_help_skips_validation() {
        // --help не требует указания режима
        let a = parse_str("-h").unwrap();
        assert!(a.help);
    }

    #[test]
    fn test_version_skips_validation() {
        let a = parse_str("-v").unwrap();
        assert!(a.version);
    }

    // ── Ошибки парсинга ──

    #[test]
    fn test_unknown_option() {
        let err = parse_str("-s --foobar").unwrap_err();
        assert!(matches!(err, ParseError::UnknownOption(_)));
    }

    #[test]
    fn test_missing_value() {
        let err = parse_str("-c").unwrap_err();
        assert!(matches!(err, ParseError::MissingValue { .. }));
    }

    #[test]
    fn test_invalid_port() {
        let err = parse_str("-s -p abc").unwrap_err();
        assert!(matches!(err, ParseError::InvalidValue { .. }));
    }

    #[test]
    fn test_invalid_interval() {
        let err = parse_str("-s -i xyz").unwrap_err();
        assert!(matches!(err, ParseError::InvalidValue { .. }));
    }

    #[test]
    fn test_invalid_bytes_suffix() {
        let err = parse_str("-c host -n 10X").unwrap_err();
        assert!(matches!(err, ParseError::InvalidValue { .. }));
    }

    // ── Display для ошибок ──

    #[test]
    fn test_error_display() {
        let err = ParseError::MissingValue {
            option: "--client".into(),
        };
        assert_eq!(
            err.to_string(),
            "option '--client' requires a value"
        );

        let err = ParseError::UnknownOption("--foo".into());
        assert_eq!(err.to_string(), "unknown option: --foo");

        let err = ParseError::Validation("test error".into());
        assert_eq!(err.to_string(), "test error");

        let err = ParseError::InvalidValue {
            option: "--port".into(),
            value: "abc".into(),
            reason: "expected u16",
        };
        assert_eq!(
            err.to_string(),
            "invalid value 'abc' for --port: expected u16"
        );
    }

    // ── Комплексные тесты ──

    #[test]
    fn test_full_client_command() {
        let a = parse_str(
            "-c 10.0.0.1 -p 5000 -u -b 10M -t 30 -l 1K -P 4 -R -w 64K -N -4 -S 32 -O 2 -T test",
        )
        .unwrap();
        assert_eq!(a.client.as_deref(), Some("10.0.0.1"));
        assert_eq!(a.port, 5000);
        assert!(a.udp);
        assert_eq!(a.bandwidth, "10M");
        assert_eq!(a.time, 30);
        assert_eq!(a.buffer_len, Some(1024));
        assert_eq!(a.parallel, 4);
        assert!(a.reverse);
        assert_eq!(a.window, Some(64 * 1024));
        assert!(a.nodelay);
        assert!(a.ipv4);
        assert_eq!(a.tos, Some(32));
        assert_eq!(a.omit, 2);
        assert_eq!(a.title.as_deref(), Some("test"));
    }

    #[test]
    fn test_full_server_command() {
        // Строчная 'g' = Gigabits, заглавная 'G' = Gigabytes
        let a = parse_str("-s -p 8080 -D -I /tmp/iperf.pid -1 -V -J -f g -i 2").unwrap();
        assert!(a.server);
        assert_eq!(a.port, 8080);
        assert!(a.daemon);
        assert_eq!(a.pidfile.as_deref(), Some("/tmp/iperf.pid"));
        assert!(a.one_off);
        assert!(a.verbose);
        assert!(a.json);
        assert_eq!(a.format, OutputFormat::Gigabits);
        assert_eq!(a.interval, 2.0);
    }

    #[test]
    fn test_equals_full_command() {
        // Строчная 'm' = Megabits, заглавная 'M' = Megabytes
        let a = parse_str("--server --port=9090 --format=m --interval=0.5").unwrap();
        assert!(a.server);
        assert_eq!(a.port, 9090);
        assert_eq!(a.format, OutputFormat::Megabits);
        assert_eq!(a.interval, 0.5);
    }

    #[test]
    fn test_long_options() {
        let a = parse_str("--client host --udp --bandwidth 100M --time 60 --parallel 8 --reverse --no-delay --version4 --zerocopy --omit 5 --title mytest --get-server-output --udp-counters-64bit").unwrap();
        assert_eq!(a.client.as_deref(), Some("host"));
        assert!(a.udp);
        assert_eq!(a.bandwidth, "100M");
        assert_eq!(a.time, 60);
        assert_eq!(a.parallel, 8);
        assert!(a.reverse);
        assert!(a.nodelay);
        assert!(a.ipv4);
        assert!(a.zerocopy);
        assert_eq!(a.omit, 5);
        assert_eq!(a.title.as_deref(), Some("mytest"));
        assert!(a.get_server_output);
        assert!(a.udp_counters_64bit);
    }
}
