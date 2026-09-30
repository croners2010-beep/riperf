
use std::io;
use std::os::unix::io::AsRawFd;

// ─────────────────────────────────────────────────────────────
// Socket options — все возвращают io::Result<()>
// ─────────────────────────────────────────────────────────────

extern "C" {
    fn setsockopt(
        fd: i32, level: i32, optname: i32,
        optval: *const std::ffi::c_void, optlen: u32,
    ) -> i32;
}

const SOL_SOCKET: i32 = 1;
const SO_SNDBUF: i32 = 7;
const SO_RCVBUF: i32 = 8;
const IPPROTO_TCP: i32 = 6;
const TCP_MAXSEG: i32 = 2;
const IPPROTO_IP: i32 = 0;
const IP_TOS: i32 = 1;

fn setsockopt_i32(fd: i32, level: i32, optname: i32, val: i32) -> io::Result<()> {
    let ret = unsafe {
        setsockopt(fd, level, optname,
            &val as *const i32 as *const std::ffi::c_void, 4)
    };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Устанавливает размер буферов отправки и приёма.
pub fn set_sock_buf<S: AsRawFd>(sock: &S, size: u32) -> io::Result<()> {
    let fd = sock.as_raw_fd();
    setsockopt_i32(fd, SOL_SOCKET, SO_SNDBUF, size as i32)?;
    setsockopt_i32(fd, SOL_SOCKET, SO_RCVBUF, size as i32)?;
    Ok(())
}

/// Устанавливает TCP Maximum Segment Size.
pub fn set_tcp_mss<S: AsRawFd>(sock: &S, mss: u32) -> io::Result<()> {
    setsockopt_i32(sock.as_raw_fd(), IPPROTO_TCP, TCP_MAXSEG, mss as i32)
}

/// Устанавливает IP Type of Service.
pub fn set_tos<S: AsRawFd>(sock: &S, tos: u32) -> io::Result<()> {
    setsockopt_i32(sock.as_raw_fd(), IPPROTO_IP, IP_TOS, tos as i32)
}

#[cfg(test)]
mod tests {
    // Тесты для socket options можно добавить позже
}
