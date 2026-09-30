# riperf

A high-performance network bandwidth measurement tool compatible with iperf3.

## Features

- **TCP and UDP testing** - Measure bandwidth over both protocols
- **Parallel streams** - Test with multiple concurrent connections (`-P`)
- **Bandwidth throttling** - Limit transmission rate (`-b`)
- **Reverse mode** - Server sends to client (`-R`)
- **JSON output** - Machine-readable results (`-J`)
- **Interval reports** - Periodic bandwidth updates (`-i`)
- **TCP options** - No-delay, MSS, window size, ToS
- **Daemon mode** - Run server as background daemon (`-D`)
- **Timestamps** - Server logs with millisecond precision
- **Zero dependencies** - Built with Rust standard library only

## Build

### Prerequisites

- Rust 2021 edition or later
- Unix-like system (Linux, macOS, BSD)

### Compilation

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test
```

The binary will be available at `target/release/riperf`.

## Quick Start

### Server

Start the server on the default port (5201):

```bash
./riperf -s
```

Expected output:
```
-----------------------------------------------------------
[10:45:27.001] Server listening on TCP port 5201
-----------------------------------------------------------
```

### Client

Run a basic TCP test to localhost:

```bash
./riperf -c localhost
```

Expected output:
```
-----------------------------------------------------------
Client connecting to localhost, TCP port 40715
-----------------------------------------------------------
[0]    0.00-2.00 sec     7.29 GBytes  29.16 Gbits/sec
[0]    2.00-4.00 sec     7.30 GBytes  29.20 Gbits/sec
[0]    4.00-6.00 sec     7.28 GBytes  29.12 Gbits/sec
[0]    6.00-8.00 sec     7.31 GBytes  29.24 Gbits/sec
[0]    8.00-10.00 sec    7.29 GBytes  29.16 Gbits/sec

--- local ---
[ ID]   Interval          Transfer      Bitrate
[0]    0.00-10.00 sec    36.47 GBytes  29.18 Gbits/sec sender
[SUM]  0.00-10.00 sec    36.47 GBytes  29.18 Gbits/sec sender
```

## Usage Examples

### Basic TCP Test

```bash
# Default 10-second test
./riperf -c 192.168.1.100

# Custom duration (30 seconds)
./riperf -c 192.168.1.100 -t 30

# Custom port
./riperf -s -p 6000
./riperf -c 192.168.1.100 -p 6000
```

### UDP Testing

```bash
# UDP test with 100 Mbps target bandwidth
./riperf -c 192.168.1.100 -u -b 100M

# UDP with custom packet size (1400 bytes)
./riperf -c 192.168.1.100 -u -b 100M -l 1400

# UDP with jitter and loss reporting
./riperf -c 192.168.1.100 -u -b 1M -i 1
```

Expected UDP output:
```
--- local ---
[ ID]   Interval          Transfer      Bitrate         Jitter     Lost/Total
[0]    0.00-5.00 sec     62.50 MBytes  100.00 Mbits/sec 0.023      15/7648 sender
```

### Parallel Streams

```bash
# 4 parallel TCP streams
./riperf -c 192.168.1.100 -P 4

# 8 parallel UDP streams
./riperf -c 192.168.1.100 -u -b 1G -P 8
```

Expected output:
```
--- local ---
[ ID]   Interval          Transfer      Bitrate
[0]    0.00-5.00 sec     19.92 GBytes  31.87 Gbits/sec sender
[1]    0.00-5.00 sec     18.83 GBytes  30.12 Gbits/sec sender
[2]    0.00-5.00 sec     18.96 GBytes  30.33 Gbits/sec sender
[3]    0.00-5.00 sec     18.97 GBytes  30.36 Gbits/sec sender
[SUM]  0.00-5.00 sec     76.68 GBytes  122.68 Gbits/sec sender
```

### Bandwidth Limiting

```bash
# Limit to 1 Gbps
./riperf -c 192.168.1.100 -b 1G

# Limit to 500 Mbps
./riperf -c 192.168.1.100 -b 500M

# UDP with 10 Mbps
./riperf -c 192.168.1.100 -u -b 10M
```

### Reverse Mode

Server sends data to client instead of receiving:

```bash
./riperf -c 192.168.1.100 -R
```

### Interval Reports

```bash
# Report every 2 seconds
./riperf -c 192.168.1.100 -i 2 -t 10
```

Expected output:
```
[0]    0.00-2.00 sec     7.29 GBytes  29.16 Gbits/sec
[0]    2.00-4.00 sec     7.30 GBytes  29.20 Gbits/sec
[0]    4.00-6.00 sec     7.28 GBytes  29.12 Gbits/sec
[0]    6.00-8.00 sec     7.31 GBytes  29.24 Gbits/sec
[0]    8.00-10.00 sec    7.29 GBytes  29.16 Gbits/sec
```

### TCP Options

```bash
# TCP no delay (disable Nagle's algorithm)
./riperf -c 192.168.1.100 -N

# Set TCP window size (64 KB)
./riperf -c 192.168.1.100 -w 64K

# Set TCP MSS (Maximum Segment Size)
./riperf -c 192.168.1.100 -M 1460

# Set IP Type of Service
./riperf -c 192.168.1.100 -S 32
```

### JSON Output

```bash
./riperf -c 192.168.1.100 -J
```

Expected output:
```json
{
  "role": "local",
  "is_sender": true,
  "is_udp": false,
  "reverse": false,
  "streams": [
    {
      "id": 0,
      "bytes": 33129365504,
      "packets": 252757,
      "duration": 10.000007,
      "jitter_ms": 0.000000,
      "lost_packets": 0,
      "total_packets": 252757
    }
  ],
  "sum": {
    "bytes": 33129365504,
    "packets": 252757,
    "duration": 10.000007,
    "jitter_ms": 0.000000,
    "lost_packets": 0,
    "total_packets": 252757
  }
}
```

### Output Formats

```bash
# Kilobits per second
./riperf -c 192.168.1.100 -f k

# Megabytes per second
./riperf -c 192.168.1.100 -f M

# Gigabits per second
./riperf -c 192.168.1.100 -f g
```

### Server Options

```bash
# Daemon mode (background)
./riperf -s -D

# Write PID file
./riperf -s -D -I /var/run/riperf.pid

# Handle one client and exit
./riperf -s -1

# Bind to specific interface
./riperf -s -B 192.168.1.10

# Custom port
./riperf -s -p 6000
```

### Client Options

```bash
# Bind to specific local interface
./riperf -c 192.168.1.100 -B 192.168.1.20

# Bind to specific client port
./riperf -c 192.168.1.100 --cport 5000

# IPv4 only
./riperf -c 192.168.1.100 -4

# IPv6 only
./riperf -c 2001:db8::1 -6

# Omit first 5 seconds from stats
./riperf -c 192.168.1.100 -O 5

# Add title prefix to output
./riperf -c 192.168.1.100 -T "Test-1"

# Get server output
./riperf -c 192.168.1.100 --get-server-output
```

### Fixed Amount Tests

```bash
# Send exactly 100 MB
./riperf -c 192.168.1.100 -n 100M

# Send exactly 10000 packets
./riperf -c 192.168.1.100 -k 10000
```

## Command Line Options

### Server or Client Options

| Option | Description |
|--------|-------------|
| `-p, --port #` | Server port (default: 5201) |
| `-f, --format [kmgKMG]` | Output format (k/m/g = K/M/Gbits, K/M/G = K/M/GBytes) |
| `-i, --interval #` | Seconds between periodic reports |
| `-F, --file name` | Transmit/receive specified file |
| `-B, --bind <host>` | Bind to specific interface |
| `-V, --verbose` | More detailed output |
| `-J, --json` | Output in JSON format |
| `--logfile f` | Send output to log file |
| `-d, --debug` | Emit debugging output |
| `-v, --version` | Show version and exit |
| `-h, --help` | Show help and exit |

### Server Options

| Option | Description |
|--------|-------------|
| `-s, --server` | Run in server mode |
| `-D, --daemon` | Run as daemon (Unix only) |
| `-I, --pidfile file` | Write PID file |
| `-1, --one-off` | Handle one client then exit |

### Client Options

| Option | Description |
|--------|-------------|
| `-c, --client <host>` | Run in client mode |
| `-u, --udp` | Use UDP instead of TCP |
| `-b, --bandwidth #[KMG][/#]` | Target bandwidth in bits/sec |
| `-t, --time #` | Duration in seconds (default: 10) |
| `-n, --bytes #[KMG]` | Number of bytes to transmit |
| `-k, --blockcount #[KMG]` | Number of packets to transmit |
| `-l, --len #[KMG]` | Buffer length |
| `--cport <port>` | Bind to specific client port |
| `-P, --parallel #` | Number of parallel streams |
| `-R, --reverse` | Reverse mode (server sends) |
| `-w, --window #[KMG]` | Socket buffer size |
| `-M, --set-mss #` | TCP maximum segment size |
| `-N, --no-delay` | Set TCP no delay |
| `-4, --version4` | IPv4 only |
| `-6, --version6` | IPv6 only |
| `-S, --tos N` | Set IP type of service |
| `-Z, --zerocopy` | Use zero copy method |
| `-O, --omit N` | Omit first N seconds |
| `-T, --title str` | Prefix output lines with title |
| `--get-server-output` | Get results from server |
| `--udp-counters-64bit` | Use 64-bit UDP counters |

## Server Log Format

The server logs events with millisecond timestamps:

```
-----------------------------------------------------------
[10:45:27.001] Server listening on TCP port 5201
-----------------------------------------------------------
[10:45:27.046] Accepted connection from 127.0.0.1, port 47652
[10:45:37.123] Client 127.0.0.1 disconnected, test finished
```

## Performance Tips

1. **Use release builds** for accurate measurements:
   ```bash
   cargo build --release
   ```

2. **Increase buffer sizes** for high-bandwidth tests:
   ```bash
   ./riperf -c host -w 4M
   ```

3. **Use parallel streams** to saturate high-bandwidth links:
   ```bash
   ./riperf -c host -P 8
   ```

4. **Disable Nagle's algorithm** for low-latency tests:
   ```bash
   ./riperf -c host -N
   ```

5. **Use UDP with bandwidth limit** for controlled tests:
   ```bash
   ./riperf -c host -u -b 1G
   ```

## Troubleshooting

### Connection refused

Make sure the server is running and the port is not blocked by a firewall:

```bash
# Check if server is listening
netstat -tlnp | grep 5201

# Test connectivity
telnet server-ip 5201
```

### Low bandwidth results

- Use release builds (`cargo build --release`)
- Increase window size (`-w`)
- Use parallel streams (`-P`)
- Check for network congestion

### High packet loss (UDP)

- Reduce bandwidth target (`-b`)
- Increase socket buffer size (`-w`)
- Check network MTU

## Comparison with iperf3

| Feature | riperf | iperf3 |
|---------|--------|--------|
| TCP testing | ✅ | ✅ |
| UDP testing | ✅ | ✅ |
| Parallel streams | ✅ | ✅ |
| Reverse mode | ✅ | ✅ |
| JSON output | ✅ | ✅ |
| Interval reports | ✅ | ✅ |
| Daemon mode | ✅ | ✅ |
| Zero dependencies | ✅ | ❌ |
| Rust implementation | ✅ | ❌ (C) |

## License

MIT License - see LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## Acknowledgments

Inspired by [iperf3](https://github.com/esnet/iperf).
