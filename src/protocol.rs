
use std::io::{Read, Write};

pub const VERSION: &str = "riperf 0.1.0";
pub const UDP_HEADER_LEN: usize = 20;

// ─────────────────────────────────────────────────────────────
// Бинарный протокол (wire format)
// ─────────────────────────────────────────────────────────────
//
// Все целые числа — big-endian.
// f64 — IEEE 754, big-endian (через to_bits/from_bits).
// Option<u64> — 1 байт флаг (0/1) + 8 байт значение (если 1).
// bool — 1 байт (0/1).
//
// TestParams (client → server, control channel):
//   u8    udp
//   u64   duration
//   opt   bytes
//   opt   blockcount
//   u64   length
//   u64   bandwidth
//   u8    reverse
//   u64   omit
//   f64   interval
//   u64   parallel
//   u8    udp_64bit
//
// Port (server → client, control channel):
//   u16   data_port
//
// TestResult (server → client, control channel):
//   u32   num_streams
//   [stream]* num_streams
//   u8    is_sender
//   u8    is_udp
//   u8    reverse
//
// StreamStats:
//   u32   stream_id
//   u64   bytes
//   u64   packets
//   f64   duration_sec
//   f64   jitter_ms
//   u64   lost_packets
//   u64   total_packets

// ─────────────────────────────────────────────────────────────
// Структуры данных
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TestParams {
    pub udp: bool,
    pub duration: u64,
    pub bytes: Option<u64>,
    pub blockcount: Option<u64>,
    pub length: usize,
    pub bandwidth: u64,
    pub reverse: bool,
    pub omit: u64,
    pub interval: f64,
    pub parallel: usize,
    pub udp_64bit: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    pub stream_id: usize,
    pub bytes: u64,
    pub packets: u64,
    pub duration_sec: f64,
    pub jitter_ms: f64,
    pub lost_packets: u64,
    pub total_packets: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TestResult {
    pub streams: Vec<StreamStats>,
    pub sum: StreamStats,
    pub is_sender: bool,
    pub is_udp: bool,
    pub reverse: bool,
}

// ─────────────────────────────────────────────────────────────
// Низкоуровневые примитивы записи
// ─────────────────────────────────────────────────────────────

fn wu8<W: Write>(w: &mut W, v: u8) -> std::io::Result<()> {
    w.write_all(&[v])
}

fn wu16<W: Write>(w: &mut W, v: u16) -> std::io::Result<()> {
    w.write_all(&v.to_be_bytes())
}

fn wu32<W: Write>(w: &mut W, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_be_bytes())
}

fn wu64<W: Write>(w: &mut W, v: u64) -> std::io::Result<()> {
    w.write_all(&v.to_be_bytes())
}

fn wf64<W: Write>(w: &mut W, v: f64) -> std::io::Result<()> {
    w.write_all(&v.to_bits().to_be_bytes())
}

fn woptu64<W: Write>(w: &mut W, v: Option<u64>) -> std::io::Result<()> {
    match v {
        Some(x) => {
            wu8(w, 1)?;
            wu64(w, x)
        }
        None => wu8(w, 0),
    }
}

fn wbool<W: Write>(w: &mut W, v: bool) -> std::io::Result<()> {
    wu8(w, if v { 1 } else { 0 })
}

// ─────────────────────────────────────────────────────────────
// Низкоуровневые примитивы чтения
// ─────────────────────────────────────────────────────────────

fn ru8<R: Read>(r: &mut R) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn ru16<R: Read>(r: &mut R) -> std::io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}

fn ru32<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn ru64<R: Read>(r: &mut R) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_be_bytes(b))
}

fn rf64<R: Read>(r: &mut R) -> std::io::Result<f64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(f64::from_bits(u64::from_be_bytes(b)))
}

fn roptu64<R: Read>(r: &mut R) -> std::io::Result<Option<u64>> {
    if ru8(r)? == 1 {
        Ok(Some(ru64(r)?))
    } else {
        Ok(None)
    }
}

fn rbool<R: Read>(r: &mut R) -> std::io::Result<bool> {
    Ok(ru8(r)? == 1)
}

// ─────────────────────────────────────────────────────────────
// Безопасные преобразования с проверкой переполнения
// ─────────────────────────────────────────────────────────────

/// Максимальный допустимый размер буфера (16 MB).
const MAX_LENGTH: u64 = 16 * 1024 * 1024;

/// Максимальное число параллельных потоков.
const MAX_PARALLEL: u64 = 128;

/// Максимальное число стримов в результате.
const MAX_STREAMS: u32 = 1024;

fn checked_usize(val: u64, name: &str, max: u64) -> std::io::Result<usize> {
    if val > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{} {} exceeds maximum {}", name, val, max),
        ));
    }
    Ok(val as usize)
}

// ─────────────────────────────────────────────────────────────
// Сериализация / десериализация TestParams
// ─────────────────────────────────────────────────────────────

pub fn write_params<W: Write>(w: &mut W, p: &TestParams) -> std::io::Result<()> {
    wbool(w, p.udp)?;
    wu64(w, p.duration)?;
    woptu64(w, p.bytes)?;
    woptu64(w, p.blockcount)?;
    wu64(w, p.length as u64)?;
    wu64(w, p.bandwidth)?;
    wbool(w, p.reverse)?;
    wu64(w, p.omit)?;
    wf64(w, p.interval)?;
    wu64(w, p.parallel as u64)?;
    wbool(w, p.udp_64bit)?;
    w.flush()
}

pub fn read_params<R: Read>(r: &mut R) -> std::io::Result<TestParams> {
    let udp = rbool(r)?;
    let duration = ru64(r)?;
    let bytes = roptu64(r)?;
    let blockcount = roptu64(r)?;
    let length_raw = ru64(r)?;
    let bandwidth = ru64(r)?;
    let reverse = rbool(r)?;
    let omit = ru64(r)?;
    let interval = rf64(r)?;
    let parallel_raw = ru64(r)?;
    let udp_64bit = rbool(r)?;

    let length = checked_usize(length_raw, "length", MAX_LENGTH)?;
    let parallel = checked_usize(parallel_raw, "parallel", MAX_PARALLEL)?;

    Ok(TestParams {
        udp,
        duration,
        bytes,
        blockcount,
        length,
        bandwidth,
        reverse,
        omit,
        interval,
        parallel,
        udp_64bit,
    })
}

// ─────────────────────────────────────────────────────────────
// Сериализация / десериализация port
// ─────────────────────────────────────────────────────────────

pub fn write_port<W: Write>(w: &mut W, port: u16) -> std::io::Result<()> {
    wu16(w, port)?;
    w.flush()
}

pub fn read_port<R: Read>(r: &mut R) -> std::io::Result<u16> {
    ru16(r)
}

// ─────────────────────────────────────────────────────────────
// Сериализация / десериализация StreamStats
// ─────────────────────────────────────────────────────────────

fn write_stream<W: Write>(w: &mut W, s: &StreamStats) -> std::io::Result<()> {
    wu32(w, s.stream_id as u32)?;
    wu64(w, s.bytes)?;
    wu64(w, s.packets)?;
    wf64(w, s.duration_sec)?;
    wf64(w, s.jitter_ms)?;
    wu64(w, s.lost_packets)?;
    wu64(w, s.total_packets)
}

fn read_stream<R: Read>(r: &mut R) -> std::io::Result<StreamStats> {
    Ok(StreamStats {
        stream_id: ru32(r)? as usize,
        bytes: ru64(r)?,
        packets: ru64(r)?,
        duration_sec: rf64(r)?,
        jitter_ms: rf64(r)?,
        lost_packets: ru64(r)?,
        total_packets: ru64(r)?,
    })
}

// ─────────────────────────────────────────────────────────────
// Сериализация / десериализация TestResult
// ─────────────────────────────────────────────────────────────

pub fn write_result<W: Write>(w: &mut W, res: &TestResult) -> std::io::Result<()> {
    wu32(w, res.streams.len() as u32)?;
    for s in &res.streams {
        write_stream(w, s)?;
    }
    wbool(w, res.is_sender)?;
    wbool(w, res.is_udp)?;
    wbool(w, res.reverse)?;
    w.flush()
}

pub fn read_result<R: Read>(r: &mut R) -> std::io::Result<TestResult> {
    let n = ru32(r)?;
    if n > MAX_STREAMS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("stream count {} exceeds maximum {}", n, MAX_STREAMS),
        ));
    }
    let mut streams = Vec::with_capacity(n as usize);
    for _ in 0..n {
        streams.push(read_stream(r)?);
    }
    let is_sender = rbool(r)?;
    let is_udp = rbool(r)?;
    let reverse = rbool(r)?;
    let sum = compute_sum(&streams);
    Ok(TestResult {
        streams,
        sum,
        is_sender,
        is_udp,
        reverse,
    })
}

// ─────────────────────────────────────────────────────────────
// Агрегация статистики
// ─────────────────────────────────────────────────────────────

/// Вычисляет суммарную статистику по всем потокам.
///
/// - `bytes`, `packets`, `lost_packets`, `total_packets` — суммируются.
/// - `duration_sec` — берётся максимум (потоки работают параллельно).
/// - `jitter_ms` — взвешенное среднее по числу пакетов каждого потока.
pub fn compute_sum(streams: &[StreamStats]) -> StreamStats {
    let mut sum = StreamStats::default();
    let mut max_dur = 0.0f64;
    let mut jitter_weighted_sum = 0.0f64;
    let mut jitter_weight_total = 0.0f64;

    for s in streams {
        sum.bytes += s.bytes;
        sum.packets += s.packets;
        sum.lost_packets += s.lost_packets;
        sum.total_packets += s.total_packets;

        if s.duration_sec > max_dur {
            max_dur = s.duration_sec;
        }

        // Взвешенное среднее jitter по числу пакетов
        if s.jitter_ms > 0.0 && s.total_packets > 0 {
            jitter_weighted_sum += s.jitter_ms * s.total_packets as f64;
            jitter_weight_total += s.total_packets as f64;
        }
    }

    sum.duration_sec = max_dur;
    if jitter_weight_total > 0.0 {
        sum.jitter_ms = jitter_weighted_sum / jitter_weight_total;
    }

    sum
}

/// Строит `TestResult` из списка статистик потоков.
pub fn build_result(
    streams: Vec<StreamStats>,
    is_sender: bool,
    is_udp: bool,
    reverse: bool,
) -> TestResult {
    let sum = compute_sum(&streams);
    TestResult {
        streams,
        sum,
        is_sender,
        is_udp,
        reverse,
    }
}

// ─────────────────────────────────────────────────────────────
// Тесты
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_params_roundtrip() {
        let params = TestParams {
            udp: true,
            duration: 30,
            bytes: Some(1_000_000),
            blockcount: None,
            length: 8192,
            bandwidth: 10_000_000,
            reverse: false,
            omit: 2,
            interval: 0.5,
            parallel: 4,
            udp_64bit: true,
        };

        let mut buf = Vec::new();
        write_params(&mut buf, &params).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let read = read_params(&mut cursor).unwrap();

        assert_eq!(read.udp, params.udp);
        assert_eq!(read.duration, params.duration);
        assert_eq!(read.bytes, params.bytes);
        assert_eq!(read.blockcount, params.blockcount);
        assert_eq!(read.length, params.length);
        assert_eq!(read.bandwidth, params.bandwidth);
        assert_eq!(read.reverse, params.reverse);
        assert_eq!(read.omit, params.omit);
        assert!((read.interval - params.interval).abs() < f64::EPSILON);
        assert_eq!(read.parallel, params.parallel);
        assert_eq!(read.udp_64bit, params.udp_64bit);
    }

    #[test]
    fn test_params_roundtrip_no_optionals() {
        let params = TestParams {
            udp: false,
            duration: 10,
            bytes: None,
            blockcount: None,
            length: 128 * 1024,
            bandwidth: 0,
            reverse: false,
            omit: 0,
            interval: 1.0,
            parallel: 1,
            udp_64bit: false,
        };

        let mut buf = Vec::new();
        write_params(&mut buf, &params).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let read = read_params(&mut cursor).unwrap();

        assert_eq!(read.udp, false);
        assert_eq!(read.bytes, None);
        assert_eq!(read.blockcount, None);
        assert_eq!(read.length, 128 * 1024);
        assert_eq!(read.bandwidth, 0);
    }

    #[test]
    fn test_port_roundtrip() {
        let mut buf = Vec::new();
        write_port(&mut buf, 5201).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_port(&mut cursor).unwrap(), 5201);
    }

    #[test]
    fn test_port_max() {
        let mut buf = Vec::new();
        write_port(&mut buf, 65535).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_port(&mut cursor).unwrap(), 65535);
    }

    #[test]
    fn test_result_roundtrip() {
        let result = TestResult {
            streams: vec![
                StreamStats {
                    stream_id: 0,
                    bytes: 1_000_000,
                    packets: 100,
                    duration_sec: 10.0,
                    jitter_ms: 0.5,
                    lost_packets: 2,
                    total_packets: 102,
                },
                StreamStats {
                    stream_id: 1,
                    bytes: 2_000_000,
                    packets: 200,
                    duration_sec: 10.5,
                    jitter_ms: 1.0,
                    lost_packets: 5,
                    total_packets: 205,
                },
            ],
            sum: StreamStats::default(), // будет пересчитан
            is_sender: true,
            is_udp: false,
            reverse: false,
        };

        let mut buf = Vec::new();
        write_result(&mut buf, &result).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let read = read_result(&mut cursor).unwrap();

        assert_eq!(read.streams.len(), 2);
        assert_eq!(read.streams[0].bytes, 1_000_000);
        assert_eq!(read.streams[1].bytes, 2_000_000);
        assert_eq!(read.is_sender, true);
        assert_eq!(read.is_udp, false);
        assert_eq!(read.reverse, false);

        // Проверяем sum
        assert_eq!(read.sum.bytes, 3_000_000);
        assert_eq!(read.sum.packets, 300);
        assert_eq!(read.sum.lost_packets, 7);
        assert_eq!(read.sum.total_packets, 307);
        assert!((read.sum.duration_sec - 10.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_sum_empty() {
        let sum = compute_sum(&[]);
        assert_eq!(sum.bytes, 0);
        assert_eq!(sum.packets, 0);
        assert_eq!(sum.duration_sec, 0.0);
        assert_eq!(sum.jitter_ms, 0.0);
    }

    #[test]
    fn test_compute_sum_weighted_jitter() {
        let streams = vec![
            StreamStats {
                stream_id: 0,
                jitter_ms: 1.0,
                total_packets: 100,
                ..Default::default()
            },
            StreamStats {
                stream_id: 1,
                jitter_ms: 3.0,
                total_packets: 300,
                ..Default::default()
            },
        ];
        let sum = compute_sum(&streams);
        // Взвешенное: (1.0*100 + 3.0*300) / (100+300) = 1000/400 = 2.5
        assert!((sum.jitter_ms - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_result() {
        let streams = vec![StreamStats {
            stream_id: 0,
            bytes: 42,
            packets: 1,
            duration_sec: 1.0,
            ..Default::default()
        }];
        let result = build_result(streams, true, false, true);
        assert_eq!(result.sum.bytes, 42);
        assert!(result.is_sender);
        assert!(!result.is_udp);
        assert!(result.reverse);
    }

    #[test]
    fn test_read_params_length_overflow() {
        // Создаём params с length > MAX_LENGTH
        let params = TestParams {
            udp: false,
            duration: 10,
            bytes: None,
            blockcount: None,
            length: (MAX_LENGTH + 1) as usize,
            bandwidth: 0,
            reverse: false,
            omit: 0,
            interval: 1.0,
            parallel: 1,
            udp_64bit: false,
        };

        let mut buf = Vec::new();
        write_params(&mut buf, &params).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let err = read_params(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_read_params_parallel_overflow() {
        let params = TestParams {
            udp: false,
            duration: 10,
            bytes: None,
            blockcount: None,
            length: 1024,
            bandwidth: 0,
            reverse: false,
            omit: 0,
            interval: 1.0,
            parallel: (MAX_PARALLEL + 1) as usize,
            udp_64bit: false,
        };

        let mut buf = Vec::new();
        write_params(&mut buf, &params).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let err = read_params(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_read_result_streams_overflow() {
        // Пишем число стримов > MAX_STREAMS
        let mut buf = Vec::new();
        wu32(&mut buf, MAX_STREAMS + 1).unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let err = read_result(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_version_constant() {
        assert!(VERSION.starts_with("riperf"));
    }

    #[test]
    fn test_udp_header_len() {
        assert_eq!(UDP_HEADER_LEN, 20);
    }
}
