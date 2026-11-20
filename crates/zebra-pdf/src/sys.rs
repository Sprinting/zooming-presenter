//! The few system calls the client and the helper need: passing a file
//! descriptor over the socket, waiting on it with a timeout, and how much
//! memory a process uses.
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// Send as much of `buf` as the socket takes now, with `fd` attached
/// (SCM_RIGHTS) when given. Returns the bytes sent.
pub fn send(sock: RawFd, buf: &[u8], fd: Option<RawFd>) -> std::io::Result<usize> {
    let mut iov = libc::iovec { iov_base: buf.as_ptr() as *mut libc::c_void, iov_len: buf.len() };
    // Room for one descriptor, aligned for cmsghdr.
    let mut space = [0u64; 8];
    // SAFETY: msghdr is plain data; zeroed is its empty value.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if let Some(fd) = fd {
        // SAFETY: CMSG_SPACE is a pure size computation.
        let need = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
        msg.msg_control = space.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = need as _;
        // SAFETY: msg_control points at `space`, which is big enough for
        // one header and one descriptor (checked by `need` <= 64).
        unsafe {
            assert!(need <= std::mem::size_of_val(&space));
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(c) as *mut RawFd, fd);
        }
    }
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    // SAFETY: `msg` and what it points at live across the call.
    let n = unsafe { libc::sendmsg(sock, &msg, flags) };
    if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
}

/// Read what is waiting (up to `buf.len()`), taking any descriptors that
/// came with it. Ok(0) is the end of the stream.
pub fn recv(sock: RawFd, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> std::io::Result<usize> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut libc::c_void, iov_len: buf.len() };
    let mut space = [0u64; 16];
    // SAFETY: as in `send`.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = space.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = std::mem::size_of_val(&space) as _;
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    // SAFETY: `msg` points at `iov`/`buf` and `space`, live across the call.
    let n = unsafe { libc::recvmsg(sock, &mut msg, flags) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: walking the control messages the kernel just filled in,
    // within msg_controllen; each SCM_RIGHTS payload is whole descriptors
    // that now belong to this process.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(c) as *const u8;
                let head = data.offset_from(c as *const u8) as usize;
                let count = ((*c).cmsg_len as usize).saturating_sub(head) / std::mem::size_of::<RawFd>();
                for i in 0..count {
                    let fd = std::ptr::read_unaligned((data as *const RawFd).add(i));
                    if fd >= 0 {
                        fds.push(OwnedFd::from_raw_fd(fd));
                    }
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    Ok(n as usize)
}

/// Wait up to `ms` for `fd` to be readable (or writable): true when it
/// is (or has hung up), false on timeout.
pub fn wait(fd: &impl AsRawFd, write: bool, ms: i32) -> bool {
    let mut p = libc::pollfd { fd: fd.as_raw_fd(), events: if write { libc::POLLOUT } else { libc::POLLIN }, revents: 0 };
    // SAFETY: one valid pollfd.
    let n = unsafe { libc::poll(&mut p, 1, ms) };
    n > 0
}

/// Memory process `pid` uses that counts against its budget: on the Mac
/// its physical footprint (what Activity Monitor shows; clean file pages,
/// such as a PDF read through the page cache, aren't in it), elsewhere
/// its anonymous resident memory. None when it can't be read.
pub fn footprint(pid: u32) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a rusage_info_v2, as RUSAGE_INFO_V2 asks for.
        let r = unsafe { libc::proc_pid_rusage(pid as i32, libc::RUSAGE_INFO_V2, &mut info as *mut _ as *mut libc::rusage_info_t) };
        (r == 0).then_some(info.ri_phys_footprint)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // statm: size resident shared … in pages; resident − shared is
        // anonymous memory (RssAnon), what the large-media tests measure.
        let s = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
        let mut it = s.split_whitespace().skip(1).map(|v| v.parse::<u64>().ok());
        let (res, shared) = (it.next()??, it.next()??);
        // SAFETY: sysconf has no preconditions.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
        Some(res.saturating_sub(shared) * page)
    }
}

/// This process's parent.
pub fn parent() -> u32 {
    // SAFETY: getppid has no preconditions.
    unsafe { libc::getppid() as u32 }
}

/// End this process now, without unwinding or running anything else
/// (the helper's watchdog).
pub fn exit_now(code: i32) -> ! {
    // SAFETY: _exit ends the process at once; nothing here relies on
    // destructors.
    unsafe { libc::_exit(code) }
}

/// Ask for a socket buffer of `bytes` (send or receive); the system may
/// give less.
pub fn buffer(sock: &impl AsRawFd, send: bool, bytes: i32) {
    let opt = if send { libc::SO_SNDBUF } else { libc::SO_RCVBUF };
    // SAFETY: a valid socket and an int option.
    unsafe { libc::setsockopt(sock.as_raw_fd(), libc::SOL_SOCKET, opt, &bytes as *const i32 as *const libc::c_void, 4) };
}
