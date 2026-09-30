
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket};
use std::os::unix::io::FromRawFd;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::cli::Args;
use crate::protocol::*;
use crate::report;
use crate::util;
use crate::Res;

// ─────────────────────────────────────────────────────────────
// Низкоуровневые сокеты (bind + connect)
// ─────────────────────────────────────────────────────────────

#[repr(C)]
struct SockaddrIn {
    sin_family: u16, sin_port: u16, sin_addr: u32, sin_zero: [u8; 8],
}

extern "C" {
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn bind(fd: i32, addr: *const SockaddrIn, len: u32) -> i32;
    fn connect(fd: i32, addr: *const SockaddrIn, len: u32) -> i32;
    #[link_name = "close"]
    fn close_fd(fd: i32) -> i32;
}

const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;

fn make_sockaddr(ip: Ipv4Addr, port: u16) -> SockaddrIn {
    SockaddrIn {
        sin_family: AF_INET as u16,
        sin_port: port.to_be(),
        sin_addr: u32::from(ip).to_be(),
        sin_zero: [0; 8],
    }
}

fn bind_and_connect(bind_ip: Ipv4Addr, bind_port: u16, target: SocketAddrV4) -> Res<TcpStream> {
    unsafe {
        let fd = socket(AF_INET, SOCK_STREAM, 0);
        if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
        let local = make_sockaddr(bind_ip, bind_port);
        if bind(fd, &local, std::mem::size_of::<SockaddrIn>() as u32) < 0 {
            let e = std::io::Error::last_os_error(); close_fd(fd); return Err(e.into());
        }
        let remote = make_sockaddr(*target.ip(), target.port());
        if connect(fd, &remote, std::mem::size_of::<SockaddrIn>() as u32) < 0 {
            let e = std::io::Error::last_os_error(); close_fd(fd); return Err(e.into());
        }
        Ok(TcpStream::from_raw_fd(fd))
    }
}

// ─────────────────────────────────────────────────────────────
// Разрешение имён
// ─────────────────────────────────────────────────────────────

fn resolve(host: &str, port: u16) -> Res<SocketAddr> {
    let target = format!("{}:{}", host, port);
    target.to_socket_addrs()?.next()
        .ok_or_else(|| format!("cannot resolve {}", target).into())
}

// ─────────────────────────────────────────────────────────────
// Построение параметров
// ─────────────────────────────────────────────────────────────

fn build_params(args: &Args) -> Res<TestParams> {
    let bytes = args.bytes;
    let blockcount = args.blockcount;
    let default_len = if args.udp { 8 * 1024 } else { 128 * 1024 };
    let length = args.buffer_len.unwrap_or(default_len) as usize;

    // Парсим bandwidth с burst (burst пока игнорируется, но парсится)
    let (mut bandwidth, _burst) = crate::cli::parse_bandwidth(&args.bandwidth)
        .map_err(|_| "invalid bandwidth format")?;

    if args.udp && bandwidth == 0 { bandwidth = 1_000_000; }

    Ok(TestParams {
        udp: args.udp, duration: args.time, bytes, blockcount, length, bandwidth,
        reverse: args.reverse, omit: args.omit, interval: args.interval,
        parallel: args.parallel.max(1), udp_64bit: args.udp_counters_64bit,
    })
}

// ─────────────────────────────────────────────────────────────
// Подключение TCP
// ─────────────────────────────────────────────────────────────

fn connect_tcp(args: &Args, data_addr: SocketAddr) -> Res<TcpStream> {
    let target_v4 = match data_addr {
        SocketAddr::V4(v4) => v4,
        _ => return Err("IPv6 not supported".into()),
    };
    let sock = if args.bind.is_some() || args.client_port.is_some() {
        let bind_ip: Ipv4Addr = args.bind.as_deref().unwrap_or("0.0.0.0")
            .parse().map_err(|_| "invalid bind address".to_string())?;
        let bind_port = args.client_port.unwrap_or(0);
        bind_and_connect(bind_ip, bind_port, target_v4)?
    } else {
        TcpStream::connect(data_addr)?
    };
    if args.nodelay { sock.set_nodelay(true)?; }
    if let Some(sz) = args.window {
        let size = u32::try_from(sz).map_err(|_| format!("window size {} exceeds u32::MAX", sz))?;
        util::set_sock_buf(&sock, size)?;
    }
    if let Some(mss) = args.mss { util::set_tcp_mss(&sock, mss)?; }
    if let Some(tos) = args.tos { util::set_tos(&sock, tos)?; }
    Ok(sock)
}

// ─────────────────────────────────────────────────────────────
// Подключение UDP
// ─────────────────────────────────────────────────────────────

fn connect_udp(args: &Args, data_addr: SocketAddr, total_streams: usize) -> Res<UdpSocket> {
    let bind_ip = args.bind.clone().unwrap_or_else(|| "0.0.0.0".to_string());
    let bind_port = if total_streams == 1 { args.client_port.unwrap_or(0) } else { 0 };
    let sock = UdpSocket::bind(format!("{}:{}", bind_ip, bind_port))?;
    sock.connect(data_addr)?;
    if let Some(sz) = args.window {
        let size = u32::try_from(sz).map_err(|_| format!("window size {} exceeds u32::MAX", sz))?;
        util::set_sock_buf(&sock, size)?;
    }
    if let Some(tos) = args.tos { util::set_tos(&sock, tos)?; }
    Ok(sock)
}

// ─────────────────────────────────────────────────────────────
// Промежуточные отчёты
// ─────────────────────────────────────────────────────────────

static REPORT_LOCK: Mutex<()> = Mutex::new(());

fn print_interval_report(
    stream_id: usize, interval_start: f64, interval_end: f64,
    interval_bytes: u64, is_udp: bool, fmt: &crate::cli::OutputFormat,
    title: Option<&str>,
) {
    let _lock = REPORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prefix = title.map(|t| format!("{} ", t)).unwrap_or_default();
    let interval_str = format!("{:.2}-{:.2} sec", interval_start, interval_end);
    let id = format!("[{}]", stream_id);
    let duration = interval_end - interval_start;
    let bitrate = if duration > 0.0 { (interval_bytes as f64 * 8.0) / duration } else { 0.0 };
    if is_udp {
        println!("{}{:<6} {:<17} {:<13} {:<15}",
            prefix, id, interval_str,
            report::format_transfer(interval_bytes),
            report::format_bitrate(bitrate, fmt));
    } else {
        println!("{}{:<6} {:<17} {:<13} {:<15}",
            prefix, id, interval_str,
            report::format_transfer(interval_bytes),
            report::format_bitrate(bitrate, fmt));
    }
}

// ─────────────────────────────────────────────────────────────
// Точка входа клиента
// ─────────────────────────────────────────────────────────────

pub fn run(args: Args, host: String) -> Res<()> {
    let ctrl_addr = resolve(&host, args.port)?;
    if args.ipv4 && ctrl_addr.is_ipv6() {
        return Err("resolved address is IPv6, but --version4 was specified".into());
    }
    if args.ipv6 && ctrl_addr.is_ipv4() {
        return Err("resolved address is IPv4, but --version6 was specified".into());
    }
    let params = build_params(&args)?;
    if args.debug { eprintln!("params: {:?}", params); }

    let mut ctrl = TcpStream::connect(ctrl_addr)?;
    write_params(&mut ctrl, &params)?;

    // Читаем data session info (port + session_id)
    let session_info = read_data_session(&mut ctrl)?;
    let data_port = session_info.port;
    let session_id = session_info.session_id;

    if args.debug { eprintln!("server data port: {}, session_id: {}", data_port, session_id); }

    // Корректно форматируем IPv6 адрес (со скобками)
    let data_addr: SocketAddr = match ctrl_addr.ip() {
        std::net::IpAddr::V4(ipv4) => format!("{}:{}", ipv4, data_port).parse()?,
        std::net::IpAddr::V6(ipv6) => format!("[{}]:{}", ipv6, data_port).parse()?,
    };

    if params.udp && params.reverse {
        return Err("reverse UDP mode is not supported".into());
    }

    println!("-----------------------------------------------------------");
    println!("Client connecting to {}, {} port {}", host,
        if params.udp { "UDP" } else { "TCP" }, data_port);
    if args.nodelay { println!("TCP no delay enabled"); }
    if let Some(mss) = args.mss { println!("TCP MSS set to {}", mss); }
    if let Some(sz) = args.window { println!("Window size: {} bytes", sz); }
    println!("-----------------------------------------------------------");

    let local_stats: Vec<StreamStats> = if params.udp {
        udp_client_send(&args, &params, data_addr, session_id)?
    } else if params.reverse {
        tcp_client_recv(&args, &params, data_addr)?
    } else {
        tcp_client_send(&args, &params, data_addr)?
    };

    let server_result: TestResult = read_result(&mut ctrl)?;
    let local_result = build_result(local_stats, !params.reverse, params.udp, params.reverse);

    if args.json {
        report::print_json("local", &local_result);
        if args.get_server_output { report::print_json("server", &server_result); }
    } else {
        report::print_result("local", &local_result, &args.format, args.title.as_deref());
        if args.get_server_output {
            report::print_result("server", &server_result, &args.format, args.title.as_deref());
        }
    }
    let _ = server_result;
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// TCP: отправка
// ─────────────────────────────────────────────────────────────

fn tcp_client_send(args: &Args, params: &TestParams, data_addr: SocketAddr) -> Res<Vec<StreamStats>> {
    let n = params.parallel.max(1);
    spawn_streams(n, |i| {
        let args = args.clone(); let params = params.clone();
        move || { let sock = connect_tcp(&args, data_addr)?; tcp_send_loop(i, sock, &params, &args, n) }
    })
}

fn tcp_send_loop(id: usize, mut sock: TcpStream, params: &TestParams, args: &Args, n: usize) -> Res<StreamStats> {
    use std::io::Write;
    let start = Instant::now();
    let buf = vec![0xABu8; params.length.max(1)];
    let mut total: u64 = 0;
    let mut packets: u64 = 0;
    let duration = Duration::from_secs(params.duration);
    let interval = params.interval;
    let mut interval_start_time = 0.0f64;
    let mut interval_start_bytes = 0u64;
    let mut next_report_time = interval;

    // Делим bandwidth на количество потоков (как в UDP)
    let per_stream_bw = if params.bandwidth > 0 {
        params.bandwidth / n as u64
    } else {
        0
    };

    loop {
        if params.bytes.is_none() && params.blockcount.is_none() && start.elapsed() >= duration { break; }
        if let Some(tb) = params.bytes { if total >= tb { break; } }
        if let Some(tk) = params.blockcount { if packets >= tk { break; } }
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
            Ok(_) => {
                total += to_send as u64;
                packets += 1;
            }
            Err(e) => return Err(e.into()),
        }
        if interval > 0.0 {
            let elapsed = start.elapsed().as_secs_f64();
            if elapsed >= next_report_time {
                let interval_bytes = total - interval_start_bytes;
                print_interval_report(id, interval_start_time, elapsed, interval_bytes, params.udp, &args.format, args.title.as_deref());
                interval_start_time = elapsed;
                interval_start_bytes = total;
                next_report_time += interval;
            }
        }
    }
    let _ = sock.shutdown(std::net::Shutdown::Write);
    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats { stream_id: id, bytes: total, packets, duration_sec: dur, total_packets: packets, ..Default::default() })
}

// ─────────────────────────────────────────────────────────────
// TCP: приём (reverse)
// ─────────────────────────────────────────────────────────────

fn tcp_client_recv(args: &Args, params: &TestParams, data_addr: SocketAddr) -> Res<Vec<StreamStats>> {
    spawn_streams(params.parallel.max(1), |i| {
        let args = args.clone(); let params = params.clone();
        move || { let sock = connect_tcp(&args, data_addr)?; tcp_recv_loop(i, sock, &params, &args) }
    })
}

fn tcp_recv_loop(id: usize, mut sock: TcpStream, params: &TestParams, args: &Args) -> Res<StreamStats> {
    let start = Instant::now();
    let mut buf = vec![0u8; 65536];
    let mut total: u64 = 0;
    let mut packets: u64 = 0;
    let interval = params.interval;
    let mut interval_start_time = 0.0f64;
    let mut interval_start_bytes = 0u64;
    let mut next_report_time = interval;

    loop {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => { total += n as u64; packets += 1; }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
        if interval > 0.0 {
            let elapsed = start.elapsed().as_secs_f64();
            if elapsed >= next_report_time {
                let interval_bytes = total - interval_start_bytes;
                print_interval_report(id, interval_start_time, elapsed, interval_bytes, params.udp, &args.format, args.title.as_deref());
                interval_start_time = elapsed;
                interval_start_bytes = total;
                next_report_time += interval;
            }
        }
    }
    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats { stream_id: id, bytes: total, packets, duration_sec: dur, total_packets: packets, ..Default::default() })
}

// ─────────────────────────────────────────────────────────────
// UDP: отправка с session_id
// ─────────────────────────────────────────────────────────────

fn udp_client_send(args: &Args, params: &TestParams, data_addr: SocketAddr, session_id: u64) -> Res<Vec<StreamStats>> {
    let n = params.parallel.max(1);
    spawn_streams(n, |i| {
        let args = args.clone(); let params = params.clone();
        move || {
            let sock = connect_udp(&args, data_addr, n)?;
            udp_send_loop(i, sock, &params, n, &args, session_id)
        }
    })
}

fn udp_send_loop(
    id: usize, sock: UdpSocket, params: &TestParams, n: usize,
    args: &Args, session_id: u64,
) -> Res<StreamStats> {
    let start = Instant::now();
    let duration = Duration::from_secs(params.duration);
    let per_stream_bw = if params.bandwidth > 0 { params.bandwidth / n as u64 } else { 0 };
    let packet_size = params.length.max(UDP_HEADER_LEN);
    let payload_size = packet_size - UDP_HEADER_LEN;
    let mut buf = vec![0u8; packet_size];
    let mut total: u64 = 0;
    let mut packets: u64 = 0;
    let mut seq: u64 = 0;
    let stream_id = id as u32;
    let interval = params.interval;
    let mut interval_start_time = 0.0f64;
    let mut interval_start_bytes = 0u64;
    let mut next_report_time = interval;

    loop {
        if params.bytes.is_none() && params.blockcount.is_none() && start.elapsed() >= duration { break; }
        if let Some(tb) = params.bytes { if total >= tb { break; } }
        if let Some(tk) = params.blockcount { if packets >= tk { break; } }

        // UDP header: session_id(8) + stream_id(4) + seq(8) + timestamp(8) = 28
        let now_us = start.elapsed().as_micros() as u64;
        buf[0..8].copy_from_slice(&session_id.to_be_bytes());
        buf[8..12].copy_from_slice(&stream_id.to_be_bytes());
        buf[12..20].copy_from_slice(&seq.to_be_bytes());
        buf[20..28].copy_from_slice(&now_us.to_be_bytes());

        match sock.send(&buf) {
            Ok(_) => {
                total += payload_size as u64;
                packets += 1;
                seq = seq.wrapping_add(1);
            }
            Err(e) => return Err(e.into()),
        }

        if per_stream_bw > 0 {
            let expected = (total as f64 * 8.0) / per_stream_bw as f64;
            let elapsed = start.elapsed().as_secs_f64();
            if expected > elapsed {
                let sleep_t = (expected - elapsed).min(0.01);
                if sleep_t > 0.0 { std::thread::sleep(Duration::from_secs_f64(sleep_t)); }
            }
        }
        if interval > 0.0 {
            let elapsed = start.elapsed().as_secs_f64();
            if elapsed >= next_report_time {
                let interval_bytes = total - interval_start_bytes;
                print_interval_report(id, interval_start_time, elapsed, interval_bytes, params.udp, &args.format, args.title.as_deref());
                interval_start_time = elapsed;
                interval_start_bytes = total;
                next_report_time += interval;
            }
        }
    }
    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats { stream_id: id, bytes: total, packets, duration_sec: dur, total_packets: packets, ..Default::default() })
}

// ─────────────────────────────────────────────────────────────
// Утилита: spawn streams
// ─────────────────────────────────────────────────────────────

fn spawn_streams<F, C>(n: usize, make_closure: F) -> Res<Vec<StreamStats>>
where F: Fn(usize) -> C, C: FnOnce() -> Res<StreamStats> + Send + 'static {
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let closure = make_closure(i);
        handles.push(std::thread::spawn(move || closure()));
    }
    let mut stats = Vec::with_capacity(n);
    let mut errors = Vec::new();

    for h in handles {
        match h.join() {
            Ok(Ok(s)) => stats.push(s),
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(_) => errors.push("thread panicked".to_string()),
        }
    }

    if !errors.is_empty() {
        return Err(format!("{} stream(s) failed: {}", errors.len(), errors.join("; ")).into());
    }

    Ok(stats)
}
