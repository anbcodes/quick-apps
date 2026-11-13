//! A small WebSocket client (RFC 6455) on its own thread.

use mhttp::{connect, read_head, Stream, Url};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

pub enum WsOut {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    Close(u16),
}

/// Kinds in `Ws` events.
pub const OPEN: u8 = 0;
pub const TEXT: u8 = 1;
pub const BINARY: u8 = 2;
pub const CLOSED: u8 = 3;
pub const ERROR: u8 = 4;

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(b: &[u8]) -> String {
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn frame(op: u8, data: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | op];
    let n = data.len();
    if n < 126 {
        f.push(0x80 | n as u8);
    } else if n < 65536 {
        f.push(0x80 | 126);
        f.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        f.push(0x80 | 127);
        f.extend_from_slice(&(n as u64).to_be_bytes());
    }
    let mut mask = [0u8; 4];
    crate::sys::random(&mut mask);
    f.extend_from_slice(&mask);
    f.extend(data.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

fn raw_fd(s: &Stream) -> RawFd {
    match s {
        Stream::Plain(t) => t.as_raw_fd(),
        Stream::Tls(t) => t.sock.as_raw_fd(),
    }
}

/// Plaintext already decrypted or buffered, so poll() would miss it.
fn buffered(r: &mut BufReader<Stream>) -> bool {
    if !r.buffer().is_empty() {
        return true;
    }
    match r.get_mut() {
        Stream::Tls(t) => t.conn.process_new_packets().map(|s| s.plaintext_bytes_to_read() > 0).unwrap_or(false),
        _ => false,
    }
}

/// Runs a socket until it closes. `emit(kind, data)` reports events; `wake_fd` signals outgoing data.
pub fn run(url: &str, headers: &[(String, String)], rx: Receiver<WsOut>, wake_fd: RawFd, emit: &dyn Fn(u8, Vec<u8>)) {
    let r = (|| -> Result<BufReader<Stream>, String> {
        let u = Url::parse(url)?;
        let mut s = connect(&u, Duration::from_secs(30))?;
        let mut key = [0u8; 16];
        crate::sys::random(&mut key);
        let mut head = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n",
            u.path,
            u.host,
            base64(&key)
        );
        for (k, v) in headers {
            if !k.contains(['\r', '\n']) && !v.contains(['\r', '\n']) {
                head += &format!("{k}: {v}\r\n");
            }
        }
        head += "\r\n";
        s.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
        let mut r = BufReader::new(s);
        let (status, _) = read_head(&mut r).map_err(|e| e.to_string())?;
        if status != 101 {
            return Err(format!("handshake failed: HTTP {status}"));
        }
        Ok(r)
    })();
    let mut r = match r {
        Ok(r) => r,
        Err(e) => return emit(ERROR, e.into_bytes()),
    };
    emit(OPEN, Vec::new());
    let mut msg: Vec<u8> = Vec::new();
    let mut msg_op = 0u8;
    loop {
        // Send everything queued.
        loop {
            match rx.try_recv() {
                Ok(WsOut::Text(d)) => {
                    let _ = r.get_mut().write_all(&frame(1, &d));
                }
                Ok(WsOut::Binary(d)) => {
                    let _ = r.get_mut().write_all(&frame(2, &d));
                }
                Ok(WsOut::Close(code)) => {
                    let _ = r.get_mut().write_all(&frame(8, &code.to_be_bytes()));
                    return emit(CLOSED, Vec::new());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let _ = r.get_mut().flush();
        if !buffered(&mut r) {
            let mut fds = [
                libc::pollfd { fd: raw_fd(r.get_ref()), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: wake_fd, events: libc::POLLIN, revents: 0 },
            ];
            unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if fds[1].revents != 0 {
                crate::sys::drain(wake_fd);
            }
            if fds[0].revents == 0 {
                continue;
            }
        }
        // Read one frame.
        let res = (|| -> std::io::Result<Option<(u8, bool, Vec<u8>)>> {
            let mut h = [0u8; 2];
            if r.fill_buf()?.is_empty() {
                return Ok(None);
            }
            r.read_exact(&mut h)?;
            let fin = h[0] & 0x80 != 0;
            let op = h[0] & 15;
            let mut n = (h[1] & 127) as u64;
            if n == 126 {
                let mut b = [0u8; 2];
                r.read_exact(&mut b)?;
                n = u16::from_be_bytes(b) as u64;
            } else if n == 127 {
                let mut b = [0u8; 8];
                r.read_exact(&mut b)?;
                n = u64::from_be_bytes(b);
            }
            if n > 16 << 20 {
                return Err(std::io::Error::other("frame too large"));
            }
            let mut mask = [0u8; 4];
            if h[1] & 0x80 != 0 {
                r.read_exact(&mut mask)?;
            }
            let mut d = vec![0u8; n as usize];
            r.read_exact(&mut d)?;
            if h[1] & 0x80 != 0 {
                for (i, b) in d.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            Ok(Some((op, fin, d)))
        })();
        match res {
            Ok(None) => return emit(CLOSED, Vec::new()),
            Err(e) => return emit(ERROR, e.to_string().into_bytes()),
            Ok(Some((op, fin, d))) => match op {
                0 | 1 | 2 => {
                    if op != 0 {
                        msg_op = op;
                        msg.clear();
                    }
                    msg.extend_from_slice(&d);
                    if msg.len() > 16 << 20 {
                        return emit(ERROR, b"message too large".to_vec());
                    }
                    if fin {
                        emit(if msg_op == 1 { TEXT } else { BINARY }, std::mem::take(&mut msg));
                    }
                }
                8 => {
                    let _ = r.get_mut().write_all(&frame(8, &d[..d.len().min(2)]));
                    return emit(CLOSED, d);
                }
                9 => {
                    let _ = r.get_mut().write_all(&frame(10, &d));
                }
                _ => {}
            },
        }
    }
}
