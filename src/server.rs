use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use crate::cli::Args;
use crate::protocol::*;
use crate::util;
use crate::Res;

extern "C" {
    fn fork() -> i32;
    fn setsid() -> i32;
    fn clock_gettime(clk_id: i32, tp: *mut Timespec) -> i32;
    fn localtime_r(timer: *const i64, result: *mut Tm) -> *mut Tm;
}

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

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
}

const CLOCK_REALTIME: i32 = 0;

/// Возвращает временную метку в формате [HH:MM:SS.mmm]
fn timestamp() -> String {
    unsafe {
        let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
        clock_gettime(CLOCK_REALTIME, &mut ts);

        let mut tm = Tm {
            tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 0,
            tm_mon: 0, tm_year: 0, tm_wday: 0, tm_yday: 0, tm_isdst: 0,
        };
        localtime_r(&ts.tv_sec, &mut tm);

        let millis = ts.tv_nsec / 1_000_000;
        format!("[{:02}:{:02}:{:02}.{:03}]", tm.tm_hour, tm.tm_min, tm.tm_sec, millis)
    }
}

// ─────────────────────────────────────────────────────────────
// Точка входа сервера
// ─────────────────────────────────────────────────────────────

pub fn run(args: Args) -> Res<()> {
    let bind = args.bind.clone().unwrap_or_else(|| "0.0.0.0".to_string());

    // Проверка IPv4/IPv6 для bind address
    if args.ipv4 && bind != "0.0.0.0" {
        let bind_ip: Ipv4Addr = bind
            .parse()
            .map_err(|_| format!("invalid bind address for IPv4: {}", bind))?;
        let _ = bind_ip; // OK, это IPv4
    }
    if args.ipv6 {
        return Err("IPv6 is not supported".into());
    }

    let addr = format!("{}:{}", bind, args.port);

    if args.daemon {
        daemonize()?;
    }

    let listener = TcpListener::bind(&addr)?;
    eprintln!("-----------------------------------------------------------");
    eprintln!("{} Server listening on TCP port {}", timestamp(), args.port);
    eprintln!("-----------------------------------------------------------");

    if let Some(ref pf) = args.pidfile {
        std::fs::write(pf, format!("{}\n", std::process::id()))?;
    }

    if args.one_off {
        let (stream, peer) = listener.accept()?;
        eprintln!("{} Accepted connection from {}, port {}", timestamp(), peer.ip(), peer.port());
        handle_control(stream, peer, &args)?;
    } else {
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => {
                    let peer = stream.peer_addr()?;
                    let ts = timestamp();
                    let args_c = args.clone();
                    std::thread::spawn(move || {
                        eprintln!(
                            "{} Accepted connection from {}, port {}",
                            ts,
                            peer.ip(),
                            peer.port()
                        );
                        if let Err(e) = handle_control(stream, peer, &args_c) {
                            eprintln!("{} connection error: {}", timestamp(), e);
                        }
                    });
                }
                Err(e) => eprintln!("{} accept error: {}", timestamp(), e),
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Демонизация
// ─────────────────────────────────────────────────────────────

fn daemonize() -> Res<()> {
    unsafe {
        let pid = fork();
        if pid < 0 {
            return Err("fork failed".into());
        }
        if pid > 0 {
            std::process::exit(0);
        }
        if setsid() < 0 {
            return Err("setsid failed".into());
        }
        let pid = fork();
        if pid < 0 {
            return Err("fork failed".into());
        }
        if pid > 0 {
            std::process::exit(0);
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// Обработка контрольного соединения
// ─────────────────────────────────────────────────────────────

fn handle_control(mut ctrl: TcpStream, peer: SocketAddr, args: &Args) -> Res<()> {
    let params = read_params(&mut ctrl)?;
    if args.debug {
        eprintln!("params: {:?}", params);
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
    let data_listener = TcpListener::bind("0.0.0.0:0")?;
    let data_port = data_listener.local_addr()?.port();
    write_port(&mut ctrl, data_port)?;

    let n = params.parallel.max(1);
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let (sock, _) = data_listener.accept()?;

        // TCP no delay
        if args.nodelay {
            let _ = sock.set_nodelay(true);
        }

        // Window / socket buffer size (уже распарсен в u64)
        if let Some(sz) = args.window {
            util::set_sock_buf(&sock, sz as u32);
        }

        // TCP max segment size
        if let Some(mss) = args.mss {
            util::set_tcp_mss(&sock, mss);
        }

        // IP Type of Service
        if let Some(tos) = args.tos {
            util::set_tos(&sock, tos);
        }

        let p = params.clone();
        handles.push(std::thread::spawn(move || -> Res<StreamStats> {
            if p.reverse {
                tcp_send_stream(i, sock, &p)
            } else {
                tcp_recv_stream(i, sock, &p)
            }
        }));
    }

    let mut stats = Vec::with_capacity(n);
    for h in handles {
        match h.join() {
            Ok(Ok(s)) => stats.push(s),
            Ok(Err(e)) => eprintln!("{} stream error: {}", timestamp(), e),
            Err(_) => eprintln!("{} thread panicked", timestamp()),
        }
    }

    // Server is sender only if reverse mode
    let result = build_result(stats, params.reverse, false, params.reverse);
    write_result(&mut ctrl, &result)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────
// TCP: проверка лимитов
// ─────────────────────────────────────────────────────────────

/// Возвращает `true`, если нужно продолжить приём/отправку.
fn should_continue(start: Instant, params: &TestParams, total: u64, packets: u64) -> bool {
    let duration = Duration::from_secs(params.duration);

    // Проверка по времени
    if params.bytes.is_none() && params.blockcount.is_none() && start.elapsed() >= duration {
        return false;
    }
    // Проверка по байтам
    if let Some(tb) = params.bytes {
        if total >= tb {
            return false;
        }
    }
    // Проверка по количеству пакетов
    if let Some(tk) = params.blockcount {
        if packets >= tk {
            return false;
        }
    }
    true
}

// ─────────────────────────────────────────────────────────────
// TCP: приём (сервер ← клиент)
// ─────────────────────────────────────────────────────────────

fn tcp_recv_stream(id: usize, mut sock: TcpStream, params: &TestParams) -> Res<StreamStats> {
    let start = Instant::now();
    let mut buf = vec![0u8; params.length.max(1)];
    let mut total: u64 = 0;
    let mut packets: u64 = 0;

    loop {
        if !should_continue(start, params, total, packets) {
            break;
        }

        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total += n as u64;
                packets += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }

    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats {
        stream_id: id,
        bytes: total,
        packets,
        duration_sec: dur,
        total_packets: packets,
        ..Default::default()
    })
}

// ─────────────────────────────────────────────────────────────
// TCP: отправка (сервер → клиент, reverse mode)
// ─────────────────────────────────────────────────────────────

fn tcp_send_stream(id: usize, mut sock: TcpStream, params: &TestParams) -> Res<StreamStats> {
    let start = Instant::now();
    let buf = vec![0xABu8; params.length.max(1)];
    let mut total: u64 = 0;
    let mut packets: u64 = 0;

    loop {
        if !should_continue(start, params, total, packets) {
            break;
        }

        // Корректировка размера последнего блока
        let mut to_send = params.length.max(1);
        if let Some(tb) = params.bytes {
            let rem = tb.saturating_sub(total);
            if rem < to_send as u64 {
                to_send = rem as usize;
                if to_send == 0 {
                    break;
                }
            }
        }

        // Throttling по bandwidth
        if params.bandwidth > 0 {
            let expected = (total as f64 * 8.0) / params.bandwidth as f64;
            let elapsed = start.elapsed().as_secs_f64();
            if expected > elapsed {
                let sleep_t = (expected - elapsed).min(0.05);
                std::thread::sleep(Duration::from_secs_f64(sleep_t));
                continue;
            }
        }

        if sock.write_all(&buf[..to_send]).is_err() {
            break;
        }
        total += to_send as u64;
        packets += 1;
    }

    let _ = sock.shutdown(std::net::Shutdown::Write);
    let dur = start.elapsed().as_secs_f64();
    Ok(StreamStats {
        stream_id: id,
        bytes: total,
        packets,
        duration_sec: dur,
        total_packets: packets,
        ..Default::default()
    })
}

// ─────────────────────────────────────────────────────────────
// UDP handler
// ─────────────────────────────────────────────────────────────

fn handle_udp(
    mut ctrl: TcpStream,
    _peer: SocketAddr,
    args: &Args,
    params: TestParams,
) -> Res<()> {
    let data_sock = UdpSocket::bind("0.0.0.0:0")?;
    let data_port = data_sock.local_addr()?.port();
    write_port(&mut ctrl, data_port)?;
    data_sock.set_read_timeout(Some(Duration::from_millis(500)))?;

    // IP Type of Service
    if let Some(tos) = args.tos {
        util::set_tos(&data_sock, tos);
    }

    let start = Instant::now();
    let mut buf = vec![0u8; 65536];
    let duration = Duration::from_secs(params.duration);
    let n = params.parallel.max(1);

    let mut bytes = vec![0u64; n];
    let mut pkts = vec![0u64; n];
    let mut max_seq = vec![0u64; n];
    let mut received: Vec<HashSet<u64>> = vec![HashSet::new(); n];
    let mut last_transit: Vec<Option<f64>> = vec![None; n];
    let mut jitter_sum = vec![0.0f64; n];
    let mut jitter_cnt = vec![0u64; n];

    let mut last_packet = Instant::now();

    loop {
        if start.elapsed() >= duration + Duration::from_secs(2) {
            break;
        }

        match data_sock.recv_from(&mut buf) {
            Ok((n_bytes, _)) => {
                if n_bytes < UDP_HEADER_LEN {
                    continue;
                }

                let sid = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
                let seq = u64::from_be_bytes(buf[4..12].try_into().unwrap());
                let ts_us = u64::from_be_bytes(buf[12..20].try_into().unwrap());
                let payload = (n_bytes - UDP_HEADER_LEN) as u64;

                if sid >= n {
                    continue;
                }

                bytes[sid] += payload;
                pkts[sid] += 1;

                if seq > max_seq[sid] {
                    max_seq[sid] = seq;
                }
                received[sid].insert(seq);

                // Вычисление jitter (в микросекундах, потом конвертируем в миллисекунды)
                let now_us = start.elapsed().as_micros() as f64;
                let transit = now_us - ts_us as f64;
                if let Some(last) = last_transit[sid] {
                    jitter_sum[sid] += (transit - last).abs();
                    jitter_cnt[sid] += 1;
                }
                last_transit[sid] = Some(transit);

                last_packet = Instant::now();
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if params.bytes.is_none()
                    && params.blockcount.is_none()
                    && last_packet.elapsed() > Duration::from_secs(2)
                {
                    break;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }

    let dur = start.elapsed().as_secs_f64();
    let mut stats = Vec::with_capacity(n);

    for i in 0..n {
        let total_pkts = max_seq[i] + 1;
        let recv = received[i].len() as u64;
        let lost = total_pkts.saturating_sub(recv);

        // Jitter: среднее абсолютное отклонение transit time, конвертируем из мкс в мс
        let jitter = if jitter_cnt[i] > 0 {
            (jitter_sum[i] / jitter_cnt[i] as f64) / 1000.0
        } else {
            0.0
        };

        stats.push(StreamStats {
            stream_id: i,
            bytes: bytes[i],
            packets: pkts[i],
            duration_sec: dur,
            jitter_ms: jitter,
            lost_packets: lost,
            total_packets: total_pkts,
        });
    }

    let result = build_result(stats, false, true, false);
    write_result(&mut ctrl, &result)?;
    Ok(())
}
