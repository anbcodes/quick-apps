//! The Wayland wire protocol: framing, argument encoding and fd passing.

use std::collections::VecDeque;
use std::io;
use std::os::fd::RawFd;

pub enum Arg<'a> {
    U(u32),
    I(i32),
    F(f64),
    S(&'a str),
    /// An object id (0 for null).
    O(u32),
    /// A new object id.
    N(u32),
    A(&'a [u8]),
    Fd(RawFd),
}

pub struct Conn {
    pub fd: RawFd,
    out: Vec<u8>,
    out_fds: Vec<RawFd>,
    inb: Vec<u8>,
    in_pos: usize,
    pub in_fds: VecDeque<RawFd>,
    next_id: u32,
    free_ids: Vec<u32>,
}

fn err() -> io::Error {
    io::Error::last_os_error()
}

impl Drop for Conn {
    /// Closing the socket makes the compositor destroy everything the
    /// connection made, so the window goes away.
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
        for fd in self.in_fds.drain(..) {
            unsafe { libc::close(fd) };
        }
    }
}

impl Conn {
    pub fn connect() -> io::Result<Conn> {
        if let Some(fd) = std::env::var("WAYLAND_SOCKET").ok().and_then(|s| s.parse::<i32>().ok()) {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
            return Ok(Self::from_fd(fd));
        }
        let name = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
        let path = if name.starts_with('/') {
            name
        } else {
            let dir = std::env::var("XDG_RUNTIME_DIR")
                .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
            format!("{dir}/{name}")
        };
        unsafe {
            let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
            if fd < 0 {
                return Err(err());
            }
            let mut addr: libc::sockaddr_un = std::mem::zeroed();
            addr.sun_family = libc::AF_UNIX as _;
            let b = path.as_bytes();
            if b.len() >= addr.sun_path.len() {
                libc::close(fd);
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "socket path too long"));
            }
            for (i, c) in b.iter().enumerate() {
                addr.sun_path[i] = *c as _;
            }
            let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
            if libc::connect(fd, &addr as *const _ as *const libc::sockaddr, len) < 0 {
                let e = err();
                libc::close(fd);
                return Err(io::Error::new(e.kind(), format!("cannot connect to {path}: {e}")));
            }
            Ok(Self::from_fd(fd))
        }
    }

    fn from_fd(fd: RawFd) -> Conn {
        Conn {
            fd,
            out: Vec::with_capacity(4096),
            out_fds: Vec::new(),
            inb: Vec::with_capacity(4096),
            in_pos: 0,
            in_fds: VecDeque::new(),
            next_id: 2,
            free_ids: Vec::new(),
        }
    }

    pub fn new_id(&mut self) -> u32 {
        self.free_ids.pop().unwrap_or_else(|| {
            let id = self.next_id;
            self.next_id += 1;
            id
        })
    }

    pub fn free_id(&mut self, id: u32) {
        self.free_ids.push(id);
    }

    pub fn send(&mut self, obj: u32, op: u16, args: &[Arg]) {
        let start = self.out.len();
        self.out.extend_from_slice(&obj.to_ne_bytes());
        self.out.extend_from_slice(&[0; 4]);
        for a in args {
            match *a {
                Arg::U(v) | Arg::O(v) | Arg::N(v) => self.out.extend_from_slice(&v.to_ne_bytes()),
                Arg::I(v) => self.out.extend_from_slice(&v.to_ne_bytes()),
                Arg::F(v) => self.out.extend_from_slice(&((v * 256.0).round() as i32).to_ne_bytes()),
                Arg::S(s) => {
                    self.out.extend_from_slice(&(s.len() as u32 + 1).to_ne_bytes());
                    self.out.extend_from_slice(s.as_bytes());
                    self.out.push(0);
                    self.pad();
                }
                Arg::A(b) => {
                    self.out.extend_from_slice(&(b.len() as u32).to_ne_bytes());
                    self.out.extend_from_slice(b);
                    self.pad();
                }
                Arg::Fd(fd) => self.out_fds.push(fd),
            }
        }
        let size = (self.out.len() - start) as u32;
        let word = (size << 16) | op as u32;
        self.out[start + 4..start + 8].copy_from_slice(&word.to_ne_bytes());
    }

    fn pad(&mut self) {
        while self.out.len() % 4 != 0 {
            self.out.push(0);
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        let mut sent = 0;
        while sent < self.out.len() {
            let mut iov = libc::iovec {
                iov_base: self.out[sent..].as_ptr() as *mut _,
                iov_len: self.out.len() - sent,
            };
            let mut cmsg = [0u64; 32];
            let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            if !self.out_fds.is_empty() {
                let n = self.out_fds.len().min(28);
                let bytes = n * 4;
                unsafe {
                    msg.msg_control = cmsg.as_mut_ptr() as *mut _;
                    msg.msg_controllen = libc::CMSG_SPACE(bytes as u32) as _;
                    let h = libc::CMSG_FIRSTHDR(&msg);
                    (*h).cmsg_level = libc::SOL_SOCKET;
                    (*h).cmsg_type = libc::SCM_RIGHTS;
                    (*h).cmsg_len = libc::CMSG_LEN(bytes as u32) as _;
                    std::ptr::copy_nonoverlapping(
                        self.out_fds.as_ptr() as *const u8,
                        libc::CMSG_DATA(h),
                        bytes,
                    );
                }
            }
            let n = unsafe { libc::sendmsg(self.fd, &msg, libc::MSG_NOSIGNAL) };
            if n < 0 {
                let e = err();
                match e.raw_os_error() {
                    Some(libc::EINTR) => continue,
                    Some(libc::EAGAIN) => {
                        let mut p = libc::pollfd { fd: self.fd, events: libc::POLLOUT, revents: 0 };
                        unsafe { libc::poll(&mut p, 1, -1) };
                        continue;
                    }
                    _ => return Err(e),
                }
            }
            let nf = self.out_fds.len().min(28);
            self.out_fds.drain(..nf);
            sent += n as usize;
        }
        self.out.clear();
        Ok(())
    }

    /// Reads what is available without blocking. Ok(0) means nothing was ready.
    pub fn read(&mut self) -> io::Result<usize> {
        if self.in_pos > 0 {
            self.inb.drain(..self.in_pos);
            self.in_pos = 0;
        }
        let mut buf = [0u8; 4096];
        let mut cmsg = [0u64; 64];
        let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut _, iov_len: buf.len() };
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg.as_mut_ptr() as *mut _;
        msg.msg_controllen = std::mem::size_of_val(&cmsg) as _;
        let n = unsafe { libc::recvmsg(self.fd, &mut msg, libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC) };
        if n < 0 {
            let e = err();
            return match e.raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => Ok(0),
                _ => Err(e),
            };
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "compositor closed the connection"));
        }
        unsafe {
            let mut h = libc::CMSG_FIRSTHDR(&msg);
            while !h.is_null() {
                if (*h).cmsg_level == libc::SOL_SOCKET && (*h).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(h) as *const i32;
                    let count = ((*h).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / 4;
                    for i in 0..count {
                        self.in_fds.push_back(std::ptr::read_unaligned(data.add(i)));
                    }
                }
                h = libc::CMSG_NXTHDR(&msg, h);
            }
        }
        self.inb.extend_from_slice(&buf[..n as usize]);
        Ok(n as usize)
    }

    /// The next complete message: (object, opcode, body).
    pub fn next(&mut self) -> Option<(u32, u16, Vec<u8>)> {
        let b = &self.inb[self.in_pos..];
        if b.len() < 8 {
            return None;
        }
        let obj = u32::from_ne_bytes(b[0..4].try_into().unwrap());
        let word = u32::from_ne_bytes(b[4..8].try_into().unwrap());
        let size = (word >> 16) as usize;
        if size < 8 || b.len() < size {
            return None;
        }
        let body = b[8..size].to_vec();
        self.in_pos += size;
        Some((obj, (word & 0xffff) as u16, body))
    }
}

/// Reads event arguments in order.
pub struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Rd { b, p: 0 }
    }
    pub fn u32(&mut self) -> u32 {
        let v = self.b.get(self.p..self.p + 4).map(|s| u32::from_ne_bytes(s.try_into().unwrap())).unwrap_or(0);
        self.p += 4;
        v
    }
    pub fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    pub fn fixed(&mut self) -> f64 {
        self.i32() as f64 / 256.0
    }
    pub fn arr(&mut self) -> &'a [u8] {
        let len = self.u32() as usize;
        let s = self.b.get(self.p..self.p + len).unwrap_or(&[]);
        self.p += (len + 3) & !3;
        s
    }
    pub fn str(&mut self) -> &'a str {
        let a = self.arr();
        let a = a.strip_suffix(&[0]).unwrap_or(a);
        std::str::from_utf8(a).unwrap_or("")
    }
}
