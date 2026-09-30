
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cli::Args;
use crate::protocol::{
    build_result, read_params, write_data_session, write_result,
    DataSessionInfo, StreamStats, TestParams, UDP_HEADER_LEN,
};
use crate::util;
use crate::Res;

// ─────────────────────────────────────────────────────────────
// FFI (Unix only)
// ─────────────────────────────────────────────────────────────

#[cfg(unix)]
extern "C" {
    fn fork() -> i32;
    fn setsid() -> i32;
    fn clock_gettime(clk_id: i32, tp: *mut Timespec) -> i32;
    fn localtime_r(timer: *const i64, result: *mut Tm) -> *mut Tm;
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(unix)]
#[repr(C)]
struct Timespec { tv_sec: i64, tv_nsec: i64 }

#[cfg(unix)]
#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

#[cfg(unix)]
const CLOCK_REALTIME: i32 = 0;

#[cfg(unix)]
fn timestamp() -> String {
    unsafe {
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        if clock_gettime(CLOCK_REALTIME, &mut ts) != 0 {
            return "[??:??:??.???]".to_string();
        }
        let mut tm = Tm {
            tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 0,
            tm_mon: 0, tm_year: 0, tm_wday: 0, tm_yday: 0, tm_isdst: 0,
            tm_gmtoff: 0, tm_zone: std::ptr::null(),
        };
        if localtime_r(&ts.tv_sec, &mut tm).is_null() {
            return "[??:??:??.???]".to_string();
        }
        format!("[{:02}:{:02}:{:02}.{:03}]",
            tm.tm_hour, tm.tm_min, tm.tm_sec, ts.tv_nsec / 1_000_000)
    }
}

#[cfg(not(unix))]
fn timestamp() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let s = now.as_secs();
    format!("[{:02}:{:02}:{:02}.{:03}]", (s/3600)%24, (s/60)%60, s%60, now.subsec_millis())
}

// ─────────────────────────────────────────────────────────────
// Константы
// ─────────────────────────────────────────────────────────────

const ACCEPT_TIMEOUT_SECS: u64 = 60;
const TCP_ACCEPT_TIMEOUT_SECS: u64 = 10;
const TCP_READ_TIMEOUT_SECS: u64 = 10;
const TCP_WRITE_TIMEOUT_SECS: u64 = 10;
const UDP_PRE_FIRST_PACKET_TIMEOUT_SECS: u64 = 10;
const UDP_IDLE_TIMEOUT_SECS: u64 = 30;

/// Максимальный размер скользящего окна sequence numbers
const MAX_SEQ_WINDOW: u64 = 1_000_000;

/// Максимальный размер TCP буфера (16 MB)
const MAX_TCP_LENGTH: usize = 16 * 1024 * 1024;

/// Максимальный размер UDP datagram (IPv4: 65535 - 20 IP - 8 UDP = 65507)
const MAX_UDP_DATAGRAM: usize = 65_507;

/// Максимальное число параллельных потоков
const MAX_PARALLEL: usize = 128;

/// Максимальная длительность теста (24 часа)
const MAX_DURATION: u64 = 86400;

/// Grace period после duration для bytes/blockcount режима
const GRACE_PERIOD_SECS: u64 = 30;

// ─────────────────────────────────────────────────────────────
// Guard для pidfile
// ─────────────────────────────────────────────────────────────

struct PidfileGuard {
    path: String,
}

impl PidfileGuard {
    fn create(path: &str) -> Res<Self> {
        let pid = std::process::id();

        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                writeln!(f, "{}", pid)?;
                Ok(PidfileGuard { path: path.to_string() })
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Проверяем, жив ли процесс из pidfile
                if let Ok(content) = std::fs::read_to_string(path) {
                    if let Ok(old_pid) = content.trim().parse::<u32>() {
                        #[cfg(unix)]
                        {
                            let ret = unsafe { kill(old_pid as i32, 0) };
                            if ret == 0 {
                                // Процесс существует и доступен
                                return Err(format!(
                                    "pidfile {} exists and process {} is still running",
                                    path, old_pid
                                ).into());
                            }
                            // Проверяем тип ошибки через ErrorKind
                            let err = std::io::Error::last_os_error();
                            if err.kind() == std::io::ErrorKind::PermissionDenied {
                                // EPERM — процесс существует, но нет прав
                                return Err(format!(
                                    "pidfile {} exists and process {} exists but cannot be inspected",
                                    path, old_pid
                                ).into());
                            }
                            // ESRCH — процесс не существует, stale pidfile
                            // Другие ошибки — не удаляем pidfile (процесс может существовать)
                            if err.kind() != std::io::ErrorKind::NotFound {
                                return Err(format!(
                                    "pidfile {} exists, cannot verify process {}: {}",
                                    path, old_pid, err
                                ).into());
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            // На non-Unix системах не пытаемся автоматически определять stale pidfile
                            return Err(format!(
                                "pidfile {} already exists; remove it manually",
                                path
                            ).into());
                        }
                    }
                }
                // Stale pidfile (ESRCH) — удаляем и создаём заново
                let _ = std::fs::remove_file(path);
                let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
                writeln!(f, "{}", pid)?;
                Ok(PidfileGuard { path: path.to_string() })
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for PidfileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ─────────────────────────────────────────────────────────────
// Guard для active_workers
// ─────────────────────────────────────────────────────────────

struct WorkerGuard {
    counter: Arc<AtomicUsize>,
}

impl WorkerGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        WorkerGuard { counter }
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

// ─────────────────────────────────────────────────────────────
// SeqTracker — скользящее окно с накоплением статистики
// ─────────────────────────────────────────────────────────────

struct SeqTracker {
    min_seq: u64,
    max_seq: u64,
    received: HashSet<u64>,
    truncated: bool,
}

impl SeqTracker {
    fn new() -> Self {
        SeqTracker {
            min_seq: u64::MAX,
            max_seq: 0,
            received: HashSet::new(),
            truncated: false,
        }
    }

    fn insert(&mut self, seq: u64) {
        if seq < self.min_seq {
            self.min_seq = seq;
        }
        if seq > self.max_seq {
            self.max_seq = seq;
        }

        // Ограничиваем размер HashSet для защиты от DoS
        if self.received.len() < MAX_SEQ_WINDOW as usize {
            self.received.insert(seq);
        } else {
            self.truncated = true;
        }
    }

    /// Возвращает статистику: (received, expected, truncated)
    fn stats(&self) -> (u64, u64, bool) {
        if self.min_seq == u64::MAX {
            return (0, 0, self.truncated);
        }

        let expected = self.max_seq.saturating_sub(self.min_seq).saturating_add(1);
        let received = self.received.len() as u64;
        (received, expected, self.truncated)
    }
}

// ─────────────────────────────────────────────────────────────
// Точка входа сервера
// ─────────────────────────────────────────────────────────────

pub fn run(args: Args) -> Res<()> {
    let bind_addr: SocketAddr = if let Ok(ipv4) = args.bind.as_deref().unwrap_or("0.0.0.0").parse::<Ipv4Addr>() {
        SocketAddr::from((ipv4, args.port))
    } else {
        return Err(format!("invalid IPv4 bind address: {}",
            args.bind.as_deref().unwrap_or("0.0.0.0")).into());
    };

    if args.ipv6 {
        return Err("IPv6 is not supported".into());
    }

    #[cfg(unix)]
    if args.daemon {
        daemonize()?;
    }

    #[cfg(not(unix))]
    if args.daemon {
        return Err("Daemon mode is only supported on Unix systems".into());
    }

    let listener = TcpListener::bind(bind_addr)?;
    eprintln!("-----------------------------------------------------------");
    eprintln!("{} Server listening on TCP port {} (timeout: {}s)", timestamp(), args.port, ACCEPT_TIMEOUT_SECS);
    eprintln!("-----------------------------------------------------------");

    // Pidfile создаётся ПОСЛЕ успешного bind
    let _pidfile_guard = if let Some(ref pf) = args.pidfile {
        Some(PidfileGuard::create(pf)?)
    } else {
        None
    };

    let active_workers = Arc::new(AtomicUsize::new(0));

    if args.one_off {
        // One-off: timeout на ожидание подключения
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + Duration::from_secs(ACCEPT_TIMEOUT_SECS);

        loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    stream.set_nonblocking(false)?;
                    eprintln!("{} Accepted connection from {}, port {}",
                        timestamp(), peer.ip(), peer.port());
                    let _wg = WorkerGuard::new(active_workers.clone());
                    let bw = bind_addr.ip();
                    handle_control(stream, peer, &args, bw)?;
                    // _wg drop — счётчик уменьшается гарантированно
                    break;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        eprintln!("{} No connection within {} seconds, exiting",
                            timestamp(), ACCEPT_TIMEOUT_SECS);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e.into()),
            }
        }
    } else {
        // Обычный режим: цикл с таймаутом 60 секунд, но НЕ завершается
        // если есть активные worker-потоки
        listener.set_nonblocking(true)?;
        let mut deadline = Instant::now() + Duration::from_secs(ACCEPT_TIMEOUT_SECS);

        loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    // При каждом подключении сбрасываем таймаут
                    deadline = Instant::now() + Duration::from_secs(ACCEPT_TIMEOUT_SECS);
                    let ts = timestamp();
                    let args_c = args.clone();
                    let bw = bind_addr.ip();
                    let aw = active_workers.clone();
                    std::thread::spawn(move || {
                        let _wg = WorkerGuard::new(aw);
                        eprintln!("{} Accepted connection from {}, port {}",
                            ts, peer.ip(), peer.port());
                        if let Err(e) = handle_control(stream, peer, &args_c, bw) {
                            eprintln!("{} connection error: {}", timestamp(), e);
                        }
                        // _wg drop — счётчик уменьшается гарантированно
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        // Проверяем, есть ли активные worker-потоки
                        if active_workers.load(Ordering::Relaxed) > 0 {
                            // Есть активные тесты — продлеваем таймаут и ждём
                            deadline = Instant::now() + Duration::from_secs(ACCEPT_TIMEOUT_SECS);
                            std::thread::sleep(Duration::from_millis(500));
                            continue;
                        }
                        // Нет активных workers — завершаемся
                        eprintln!("{} No connection within {} seconds, exiting",
                            timestamp(), ACCEPT_TIMEOUT_SECS);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => eprintln!("{} accept error: {}", timestamp(), e),
            }
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Демонизация (только Unix)
// ─────────────────────────────────────────────────────────────

#[cfg(unix)]
fn daemonize() -> Res<()> {
    unsafe {
        let pid = fork();
        if pid < 0 { return Err("fork failed".into()); }
        if pid > 0 { std::process::exit(0); }
        if setsid() < 0 { return Err("setsid failed".into()); }
        let pid = fork();
        if pid < 0 { return Err("fork failed".into()); }
        if pid > 0 { std::process::exit(0); }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Обработка контрольного соединения
// ─────────────────────────────────────────────────────────────

fn handle_control(
    mut ctrl: TcpStream,
    peer: SocketAddr,
    args: &Args,
    _bind_ip: std::net::IpAddr,
) -> Res<()> {
    ctrl.set_read_timeout(Some(Duration::from_secs(TCP_READ_TIMEOUT_SECS)))?;

    let params = read_params(&mut ctrl)?;

    // Валидация параметров теста
    if params.length == 0 {
        return Err("length must be greater than zero".into());
    }
    if params.parallel == 0 || params.parallel > MAX_PARALLEL {
        return Err(format!("parallel must be in range 1..={}", MAX_PARALLEL).into());
    }
    if params.duration > MAX_DURATION {
        return Err(format!("requested duration {}s exceeds maximum {}s",
            params.duration, MAX_DURATION).into());
    }

    // Проверка размера буфера
    if params.udp {
        // params.length — полный размер UDP датаграммы (включая заголовок)
        if params.length > MAX_UDP_DATAGRAM {
            return Err(format!("UDP datagram size {} exceeds maximum {}",
                params.length, MAX_UDP_DATAGRAM).into());
        }
    } else if params.length > MAX_TCP_LENGTH {
        return Err(format!("TCP buffer {} exceeds maximum {}",
            params.length, MAX_TCP_LENGTH).into());
    }

    if args.debug {
        eprintln!("params: {:?}", params);
    }

    if params.udp && params.reverse {
        return Err("UDP reverse mode is not supported".into());
    }

    let result = if params.udp {
        handle_udp(ctrl, peer, args, params)
    } else {
        handle_tcp(ctrl, peer, args, params)
    };

    match &result {
        Ok(_) => eprintln!("{} Client {} disconnected, test finished", timestamp(), peer.ip()),
        Err(e) => eprintln!("{} Client {} error: {}", timestamp(), peer.ip(), e),
    }
    result
}

// ─────────────────────────────────────────────────────────────
// TCP handler
// ─────────────────────────────────────────────────────────────

fn handle_tcp(
    mut ctrl: TcpStream,
    _peer: SocketAddr,
    args: &Args,
    params: TestParams,
) -> Res<()> {
    let local_addr = ctrl.local_addr()?;
    let data_listener = TcpListener::bind(SocketAddr::new(local_addr.ip(), 0))?;
    let data_port = data_listener.local_addr()?.port();

    // Генерируем session_id (для TCP не используется, но отправляем для единообразия)
    let session_id = generate_session_id();
    let session_info = DataSessionInfo { port: data_port, session_id };

    ctrl.set_write_timeout(Some(Duration::from_secs(TCP_WRITE_TIMEOUT_SECS)))?;
    write_data_session(&mut ctrl, &session_info)?;

    data_listener.set_nonblocking(true)?;
    let tcp_accept_deadline = Instant::now()
        .checked_add(Duration::from_secs(TCP_ACCEPT_TIMEOUT_SECS))
        .ok_or("TCP accept deadline overflow")?;

    let n = params.parallel;
    let mut handles = Vec::with_capacity(n);
    let mut accepted = 0usize;

    while accepted < n {
        match data_listener.accept() {
            Ok((sock, _)) => {
                let stream_id = accepted;
                accepted += 1;

                sock.set_read_timeout(Some(Duration::from_secs(TCP_READ_TIMEOUT_SECS)))?;
                sock.set_write_timeout(Some(Duration::from_secs(TCP_WRITE_TIMEOUT_SECS)))?;

                // Ошибки socket options НЕ игнорируются
                if args.nodelay {
                    sock.set_nodelay(true)?;
                }
                if let Some(sz) = args.window {
                    let size = u32::try_from(sz)
                        .map_err(|_| format!("window size {} exceeds u32::MAX", sz))?;
                    util::set_sock_buf(&sock, size)?;
                }
                if let Some(mss) = args.mss {
                    util::set_tcp_mss(&sock, mss)?;
                }
                if let Some(tos) = args.tos {
                    util::set_tos(&sock, tos)?;
                }

                let p = params.clone();
                let n_streams = n;
                handles.push(std::thread::spawn(move || -> Res<StreamStats> {
                    if p.reverse { tcp_send_stream(stream_id, sock, &p, n_streams) }
                    else { tcp_recv_stream(stream_id, sock, &p) }
                }));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= tcp_accept_deadline {
                    drop(data_listener);
                    return Err(format!("TCP data connection timeout: accepted {}/{} streams within {}s",
                        accepted, n, TCP_ACCEPT_TIMEOUT_SECS).into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                drop(data_listener);
                return Err(e.into());
            }
        }
    }

    drop(data_listener);

    let mut stats = Vec::with_capacity(n);
    let mut errors: Vec<String> = Vec::new();

    for h in handles {
        match h.join() {
            Ok(Ok(s)) => stats.push(s),
            Ok(Err(e)) => {
                let msg = e.to_string();
                eprintln!("{} stream error: {}", timestamp(), msg);
                errors.push(msg);
            }
            Err(_) => {
                eprintln!("{} thread panicked", timestamp());
                errors.push("thread panicked".to_string());
            }
        }
    }

    if !errors.is_empty() {
        return Err(format!("{} of {} stream(s) failed: {}",
            errors.len(), n, errors.join("; ")).into());
    }

    let result = build_result(stats, params.reverse, false, params.reverse);
    write_result(&mut ctrl, &result)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// TCP: проверка лимитов с hard deadline
// ─────────────────────────────────────────────────────────────

fn should_continue(
    start: Instant,
    hard_deadline: Instant,
    params: &TestParams,
    total: u64,
) -> bool {
    // Hard deadline — абсолютная верхняя граница
    if Instant::now() >= hard_deadline {
        return false;
    }
    // По времени (только если нет bytes-лимит)
    let duration = Duration::from_secs(params.duration);
    if params.bytes.is_none() && start.elapsed() >= duration {
        return false;
    }
    // По байтам
    if let Some(tb) = params.bytes {
        if total >= tb { return false; }
    }
    true
}

// ─────────────────────────────────────────────────────────────
// TCP: приём
// ─────────────────────────────────────────────────────────────

fn tcp_recv_stream(id: usize, mut sock: TcpStream, params: &TestParams) -> Res<StreamStats> {
    let start = Instant::now();
    // Для bytes/blockcount режимов не ограничиваем время жёстко
    let hard_deadline = if params.bytes.is_some() || params.blockcount.is_some() {
        start.checked_add(Duration::from_secs(MAX_DURATION)).ok_or("deadline overflow")?
    } else {
        start
            .checked_add(Duration::from_secs(params.duration))
            .and_then(|t| t.checked_add(Duration::from_secs(GRACE_PERIOD_SECS)))
            .ok_or("test deadline overflow")?
    };
    let mut buf = vec![0u8; params.length.max(1)];
    let mut total: u64 = 0;

    loop {
        if !should_continue(start, hard_deadline, params, total) { break; }
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => { total += n as u64; }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut
                   || e.kind() == std::io::ErrorKind::WouldBlock => {
                return Err("TCP receive timeout".into());
            }
            Err(e) => return Err(e.into()),
        }
    }

    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats {
        stream_id: id, bytes: total, packets: 0,
        duration_sec: dur, total_packets: 0, ..Default::default()
    })
}

// ─────────────────────────────────────────────────────────────
// TCP: отправка (reverse mode)
// ─────────────────────────────────────────────────────────────

fn tcp_send_stream(id: usize, mut sock: TcpStream, params: &TestParams, n: usize) -> Res<StreamStats> {
    let start = Instant::now();
    // Для bytes/blockcount режимов не ограничиваем время жёстко
    let hard_deadline = if params.bytes.is_some() || params.blockcount.is_some() {
        start.checked_add(Duration::from_secs(MAX_DURATION)).ok_or("deadline overflow")?
    } else {
        start
            .checked_add(Duration::from_secs(params.duration))
            .and_then(|t| t.checked_add(Duration::from_secs(GRACE_PERIOD_SECS)))
            .ok_or("test deadline overflow")?
    };
    let buf = vec![0xABu8; params.length.max(1)];
    let mut total: u64 = 0;

    // Делим bandwidth на количество потоков (как в UDP)
    let per_stream_bw = if params.bandwidth > 0 {
        params.bandwidth / n as u64
    } else {
        0
    };

    loop {
        if !should_continue(start, hard_deadline, params, total) { break; }

        let mut to_send = params.length.max(1);
        if let Some(tb) = params.bytes {
            let rem = tb.saturating_sub(total);
            if rem < to_send as u64 { to_send = rem as usize; if to_send == 0 { break; } }
        }

        if per_stream_bw > 0 {
            let expected = (total as f64 * 8.0) / per_stream_bw as f64;
            let elapsed = start.elapsed().as_secs_f64();
            if expected > elapsed {
                let sleep_t = (expected - elapsed).min(0.05);
                std::thread::sleep(Duration::from_secs_f64(sleep_t));
                continue;
            }
        }

        match sock.write_all(&buf[..to_send]) {
            Ok(_) => { total += to_send as u64; }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut
                   || e.kind() == std::io::ErrorKind::WouldBlock => {
                return Err("TCP write timeout".into());
            }
            // Не маскируем BrokenPipe/ConnectionReset — возвращаем как ошибку
            Err(e) => return Err(e.into()),
        }
    }

    let _ = sock.shutdown(std::net::Shutdown::Write);
    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats {
        stream_id: id, bytes: total, packets: 0,
        duration_sec: dur, total_packets: 0, ..Default::default()
    })
}

// ─────────────────────────────────────────────────────────────
// UDP handler
// ─────────────────────────────────────────────────────────────

fn generate_session_id() -> u64 {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Используем наносекунды + PID для уникальности
    (now.as_nanos() as u64) ^ ((std::process::id() as u64) << 32)
}

fn handle_udp(
    mut ctrl: TcpStream,
    peer: SocketAddr,
    args: &Args,
    params: TestParams,
) -> Res<()> {
    let local_addr = ctrl.local_addr()?;
    let data_sock = UdpSocket::bind(SocketAddr::new(local_addr.ip(), 0))?;
    let data_port = data_sock.local_addr()?.port();

    // Генерируем session_id для фильтрации UDP пакетов
    let session_id = generate_session_id();
    let session_info = DataSessionInfo { port: data_port, session_id };

    ctrl.set_write_timeout(Some(Duration::from_secs(TCP_WRITE_TIMEOUT_SECS)))?;
    write_data_session(&mut ctrl, &session_info)?;
    data_sock.set_read_timeout(Some(Duration::from_millis(500)))?;

    if let Some(tos) = args.tos {
        util::set_tos(&data_sock, tos)?;
    }

    let n = params.parallel;
    let duration = Duration::from_secs(params.duration);

    // Для bytes/blockcount режимов не ограничиваем время жёстко
    let max_duration = if params.bytes.is_some() || params.blockcount.is_some() {
        Duration::from_secs(MAX_DURATION) // 24 часа — разумный максимум
    } else {
        duration
            .checked_add(Duration::from_secs(2))
            .ok_or("duration overflow")?
    };

    let mut bytes = vec![0u64; n];
    let mut pkts = vec![0u64; n];
    let mut trackers: Vec<SeqTracker> = (0..n).map(|_| SeqTracker::new()).collect();
    let mut jitter = vec![0.0f64; n];
    let mut last_transit: Vec<Option<f64>> = vec![None; n];
    let mut buf = vec![0u8; 65536];

    let mut test_start: Option<Instant> = None;
    let mut last_packet: Option<Instant> = None;
    let mut wait_start: Option<Instant> = None;

    loop {
        // Hard deadline по duration
        if let Some(start) = test_start {
            if start.elapsed() >= max_duration {
                break;
            }
        }

        // Суммарная проверка bytes/blockcount (iperf-совместимо)
        if test_start.is_some() {
            let total_bytes: u64 = bytes.iter().sum();
            let total_pkts: u64 = pkts.iter().sum();

            let mut all_done = false;
            if let Some(tb) = params.bytes {
                if total_bytes >= tb { all_done = true; }
            } else if let Some(tk) = params.blockcount {
                if total_pkts >= tk { all_done = true; }
            }

            if all_done {
                break;
            }
        }

        match data_sock.recv_from(&mut buf) {
            Ok((n_bytes, src)) => {
                // Проверка IP отправителя
                if src.ip() != peer.ip() {
                    if args.debug {
                        eprintln!("Ignoring UDP packet from unexpected IP: {}", src.ip());
                    }
                    continue;
                }

                // Проверка длины (минимум UDP_HEADER_LEN)
                if n_bytes < UDP_HEADER_LEN {
                    continue;
                }

                // Парсинг заголовка: session_id(8) + stream_id(4) + seq(8) + timestamp(8)
                let sid_session_bytes: [u8; 8] = buf[0..8].try_into().map_err(|_| "invalid session_id")?;
                let sid_bytes: [u8; 4] = buf[8..12].try_into().map_err(|_| "invalid stream_id")?;
                let seq_bytes: [u8; 8] = buf[12..20].try_into().map_err(|_| "invalid seq")?;
                let ts_bytes: [u8; 8] = buf[20..28].try_into().map_err(|_| "invalid timestamp")?;

                let pkt_session_id = u64::from_be_bytes(sid_session_bytes);
                let sid = u32::from_be_bytes(sid_bytes) as usize;
                let seq = u64::from_be_bytes(seq_bytes);
                let ts_us = u64::from_be_bytes(ts_bytes);

                // Проверка session_id
                if pkt_session_id != session_id {
                    if args.debug {
                        eprintln!("Ignoring UDP packet with wrong session_id: {} (expected {})",
                            pkt_session_id, session_id);
                    }
                    continue;
                }

                // Проверка sid ДО установки test_start
                if sid >= n {
                    continue;
                }

                // Проверка timestamp (защита от некорректных значений)
                let now_us = test_start.map(|s| s.elapsed().as_micros() as i128).unwrap_or(0);
                let sent_us = ts_us as i128;
                let transit = now_us - sent_us;
                if transit.abs() > 60_000_000 {
                    // Более 60 секунд разницы — некорректный timestamp
                    if args.debug {
                        eprintln!("Ignoring UDP packet with invalid timestamp delta: {} us", transit);
                    }
                    continue;
                }

                // Только теперь устанавливаем test_start и last_packet
                let test_start_instant = *test_start.get_or_insert_with(Instant::now);
                let _ = last_packet.insert(Instant::now());

                let payload = (n_bytes - UDP_HEADER_LEN) as u64;

                bytes[sid] += payload;
                pkts[sid] += 1;

                // Скользящее окно для tracking
                trackers[sid].insert(seq);

                // RTP-style сглаженный jitter (коэффициент 1/16)
                let now_us_f64 = test_start_instant.elapsed().as_micros() as f64;
                let transit_f64 = now_us_f64 - ts_us as f64;
                if let Some(last) = last_transit[sid] {
                    let d = (transit_f64 - last).abs();
                    jitter[sid] += (d - jitter[sid]) / 16.0;
                }
                last_transit[sid] = Some(transit_f64);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                       || e.kind() == std::io::ErrorKind::TimedOut => {
                if test_start.is_none() {
                    let ws = *wait_start.get_or_insert_with(Instant::now);
                    if ws.elapsed() > Duration::from_secs(UDP_PRE_FIRST_PACKET_TIMEOUT_SECS) {
                        if args.debug {
                            eprintln!("UDP: no packets within {}s, exiting",
                                UDP_PRE_FIRST_PACKET_TIMEOUT_SECS);
                        }
                        break;
                    }
                } else if let Some(lp) = last_packet {
                    if lp.elapsed() > Duration::from_secs(UDP_IDLE_TIMEOUT_SECS) {
                        break;
                    }
                }
            }
            Err(e) => return Err(e.into()),
        }
    }

    let dur = test_start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let mut stats = Vec::with_capacity(n);

    for i in 0..n {
        // Используем статистику из SeqTracker
        let (received, expected, truncated) = trackers[i].stats();
        let lost = expected.saturating_sub(received);
        let jitter_ms = jitter[i] / 1000.0;

        if truncated && args.debug {
            eprintln!("UDP stream {}: sequence tracking was truncated (loss estimate may be inaccurate)", i);
        }

        stats.push(StreamStats {
            stream_id: i,
            bytes: bytes[i],
            packets: pkts[i],
            duration_sec: dur,
            jitter_ms,
            lost_packets: lost,
            total_packets: expected,
        });
    }

    let result = build_result(stats, false, true, false);
    write_result(&mut ctrl, &result)?;
    Ok(())
}
