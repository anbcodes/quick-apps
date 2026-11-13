//! File access through handles. Paths resolve with openat2(RESOLVE_BENEATH),
//! so nothing escapes a handle (docs/io.md).

use qa_proto::err;
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

pub const READ: u32 = 1;
pub const WRITE: u32 = 2;
pub const CREATE: u32 = 4;
pub const TRUNC: u32 = 8;
pub const APPEND: u32 = 16;
pub const EXCL: u32 = 32;

const MAX_OPEN: usize = 64;
#[cfg(target_os = "linux")]
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
#[cfg(target_os = "linux")]
const RESOLVE_BENEATH: u64 = 0x08;
// macOS has no O_PATH; a read-only open is enough to stat or to use as a parent.
#[cfg(target_os = "macos")]
const O_PATH: i32 = libc::O_RDONLY;
#[cfg(target_os = "linux")]
const O_PATH: i32 = libc::O_PATH;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HKind {
    Dir { write: bool },
    File { read: bool, write: bool },
}

pub struct Handle {
    pub fd: OwnedFd,
    pub kind: HKind,
}

struct Open {
    fd: OwnedFd,
}

#[derive(Default)]
pub struct Fs {
    handles: Vec<Option<Handle>>,
    files: Vec<Option<Open>>,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

fn errno_code(e: i32) -> i32 {
    match e {
        libc::ENOENT | libc::ENOTDIR => err::NOT_FOUND,
        libc::EEXIST | libc::ENOTEMPTY => err::EXISTS,
        libc::EACCES | libc::EPERM | libc::EROFS => err::DENIED,
        libc::EXDEV | libc::ELOOP | libc::EINVAL | libc::EISDIR => err::INVALID,
        _ => err::IO,
    }
}

fn last() -> i32 {
    errno_code(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
}

/// Rejects absolute paths, `..`, and NUL.
pub fn valid(path: &str) -> bool {
    !path.starts_with('/') && !path.contains('\0') && path.split('/').all(|c| c != "..")
}

/// Without openat2, every step of the path is opened by hand and never follows a link. `valid`
/// has already rejected `..` and absolute paths, so nothing can climb out of `dir`.
#[cfg(target_os = "macos")]
fn openat2(dir: RawFd, path: &str, flags: i32, mode: u32) -> Result<OwnedFd, i32> {
    let parts: Vec<&str> = path.split('/').filter(|c| !c.is_empty() && *c != ".").collect();
    let open1 = |at: RawFd, name: &str, fl: i32| -> Result<OwnedFd, i32> {
        let c = CString::new(name).map_err(|_| err::INVALID)?;
        let fd = unsafe { libc::openat(at, c.as_ptr(), fl | libc::O_CLOEXEC | libc::O_NOFOLLOW, mode as libc::c_uint) };
        if fd < 0 {
            return Err(last());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    };
    let Some((leaf, dirs)) = parts.split_last() else {
        return open1(dir, ".", flags);
    };
    let mut held: Option<OwnedFd> = None;
    for d in dirs {
        let at = held.as_ref().map_or(dir, |f| f.as_raw_fd());
        held = Some(open1(at, d, libc::O_RDONLY | libc::O_DIRECTORY)?);
    }
    open1(held.as_ref().map_or(dir, |f| f.as_raw_fd()), leaf, flags)
}

#[cfg(target_os = "linux")]
fn openat2(dir: RawFd, path: &str, flags: i32, mode: u32) -> Result<OwnedFd, i32> {
    let p = if path.is_empty() { "." } else { path };
    let c = CString::new(p).map_err(|_| err::INVALID)?;
    // openat2 rejects a mode unless a file may be created.
    let mode = if flags & libc::O_CREAT != 0 { mode } else { 0 };
    let how = OpenHow { flags: (flags | libc::O_CLOEXEC) as u64, mode: mode as u64, resolve: RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS };
    let fd = unsafe { libc::syscall(libc::SYS_openat2, dir, c.as_ptr(), &how as *const OpenHow, std::mem::size_of::<OpenHow>()) };
    if fd < 0 {
        return Err(last());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}

/// The entries of an open directory as (kind, name): 0 file, 1 folder, 2 other.
#[cfg(target_os = "linux")]
fn read_names(fd: OwnedFd) -> Result<Vec<(u8, String)>, i32> {
    let rd = std::fs::read_dir(format!("/proc/self/fd/{}", fd.as_raw_fd())).map_err(|_| err::IO)?;
    Ok(rd
        .flatten()
        .map(|e| {
            let k = match e.file_type() {
                Ok(t) if t.is_file() => 0,
                Ok(t) if t.is_dir() => 1,
                _ => 2,
            };
            (k, e.file_name().to_string_lossy().into_owned())
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn read_names(fd: OwnedFd) -> Result<Vec<(u8, String)>, i32> {
    let dir = unsafe { libc::fdopendir(fd.as_raw_fd()) };
    if dir.is_null() {
        return Err(err::IO);
    }
    // fdopendir owns the descriptor now and closedir closes it.
    std::mem::forget(fd);
    let mut out = Vec::new();
    loop {
        let e = unsafe { libc::readdir(dir) };
        if e.is_null() {
            break;
        }
        let e = unsafe { &*e };
        let name = unsafe { std::ffi::CStr::from_ptr(e.d_name.as_ptr()) }.to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let k = match e.d_type {
            libc::DT_REG => 0,
            libc::DT_DIR => 1,
            _ => 2,
        };
        out.push((k, name));
    }
    unsafe { libc::closedir(dir) };
    Ok(out)
}

/// Opens a directory path outside any handle (the runtime's own folders, granted folders).
pub fn open_dir(path: &Path) -> std::io::Result<OwnedFd> {
    let f = std::fs::File::open(path)?;
    if !f.metadata()?.is_dir() {
        return Err(std::io::Error::other("not a folder"));
    }
    Ok(f.into())
}

fn split(path: &str) -> (&str, &str) {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => ("", p),
    }
}

impl Fs {
    pub fn add_handle(&mut self, h: Handle) -> i32 {
        if let Some(i) = self.handles.iter().position(|h| h.is_none()).filter(|&i| i >= 2) {
            self.handles[i] = Some(h);
            return i as i32;
        }
        self.handles.push(Some(h));
        (self.handles.len() - 1) as i32
    }

    /// Handles 0 and 1 are fixed: pass None to leave one empty.
    pub fn new(data: Option<OwnedFd>, bundle: Option<OwnedFd>) -> Fs {
        let mut fs = Fs::default();
        fs.handles.push(data.map(|fd| Handle { fd, kind: HKind::Dir { write: true } }));
        fs.handles.push(bundle.map(|fd| Handle { fd, kind: HKind::Dir { write: false } }));
        fs
    }

    fn handle(&self, h: i32) -> Result<&Handle, i32> {
        self.handles.get(h as usize).and_then(|h| h.as_ref()).ok_or(err::BAD_HANDLE)
    }

    /// The directory fd and whether it is writable, for a path-taking call.
    fn dir(&self, h: i32, path: &str) -> Result<(RawFd, bool), i32> {
        let hd = self.handle(h)?;
        match hd.kind {
            HKind::Dir { write } => {
                if !valid(path) {
                    return Err(err::INVALID);
                }
                Ok((hd.fd.as_raw_fd(), write))
            }
            HKind::File { .. } => Err(err::INVALID),
        }
    }

    pub fn open(&mut self, h: i32, path: &str, flags: u32) -> i32 {
        if self.files.iter().filter(|f| f.is_some()).count() >= MAX_OPEN {
            return err::LIMIT;
        }
        let want_write = flags & (WRITE | CREATE | TRUNC | APPEND) != 0;
        let want_read = flags & READ != 0 || !want_write;
        let fd = match self.handle(h) {
            Err(e) => return e,
            Ok(Handle { fd, kind: HKind::File { read, write } }) => {
                if !path.is_empty() {
                    return err::INVALID;
                }
                if (want_write && !write) || (want_read && !read) {
                    return err::DENIED;
                }
                let d = unsafe { libc::dup(fd.as_raw_fd()) };
                if d < 0 {
                    return last();
                }
                unsafe { OwnedFd::from_raw_fd(d) }
            }
            Ok(_) => {
                let (dir, writable) = match self.dir(h, path) {
                    Ok(v) => v,
                    Err(e) => return e,
                };
                if want_write && !writable {
                    return err::DENIED;
                }
                let mut o = match (want_read, want_write) {
                    (true, true) => libc::O_RDWR,
                    (false, true) => libc::O_WRONLY,
                    _ => libc::O_RDONLY,
                };
                if flags & CREATE != 0 {
                    o |= libc::O_CREAT;
                }
                if flags & TRUNC != 0 {
                    o |= libc::O_TRUNC;
                }
                if flags & APPEND != 0 {
                    o |= libc::O_APPEND;
                }
                if flags & EXCL != 0 {
                    o |= libc::O_EXCL;
                }
                match openat2(dir, path, o, 0o644) {
                    Ok(fd) => fd,
                    Err(e) => return e,
                }
            }
        };
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } == 0 && st.st_mode & libc::S_IFMT == libc::S_IFDIR {
            return err::INVALID;
        }
        let f = Some(Open { fd });
        match self.files.iter().position(|f| f.is_none()) {
            Some(i) => {
                self.files[i] = f;
                i as i32
            }
            None => {
                self.files.push(f);
                (self.files.len() - 1) as i32
            }
        }
    }

    fn file(&self, fd: i32) -> Result<RawFd, i32> {
        self.files.get(fd as usize).and_then(|f| f.as_ref()).map(|f| f.fd.as_raw_fd()).ok_or(err::BAD_HANDLE)
    }

    pub fn read(&self, fd: i32, buf: &mut [u8]) -> i32 {
        match self.file(fd) {
            Ok(f) => {
                let n = unsafe { libc::read(f, buf.as_mut_ptr() as *mut _, buf.len().min(i32::MAX as usize)) };
                if n < 0 {
                    last()
                } else {
                    n as i32
                }
            }
            Err(e) => e,
        }
    }

    pub fn write(&self, fd: i32, buf: &[u8]) -> i32 {
        match self.file(fd) {
            Ok(f) => {
                let n = unsafe { libc::write(f, buf.as_ptr() as *const _, buf.len().min(i32::MAX as usize)) };
                if n < 0 {
                    last()
                } else {
                    n as i32
                }
            }
            Err(e) => e,
        }
    }

    pub fn seek(&self, fd: i32, off: i64, whence: u32) -> i64 {
        let w = match whence {
            0 => libc::SEEK_SET,
            1 => libc::SEEK_CUR,
            2 => libc::SEEK_END,
            _ => return err::INVALID as i64,
        };
        match self.file(fd) {
            Ok(f) => {
                let r = unsafe { libc::lseek(f, off, w) };
                if r < 0 {
                    last() as i64
                } else {
                    r
                }
            }
            Err(e) => e as i64,
        }
    }

    pub fn close(&mut self, fd: i32) -> i32 {
        match self.files.get_mut(fd as usize) {
            Some(f @ Some(_)) => {
                *f = None;
                0
            }
            _ => err::BAD_HANDLE,
        }
    }

    /// 17 bytes: u8 kind (0 file, 1 dir, 2 other), u64 size, i64 mtime_ms.
    pub fn stat(&self, h: i32, path: &str) -> Result<[u8; 17], i32> {
        let fd = match self.handle(h)?.kind {
            HKind::File { .. } if path.is_empty() => unsafe { OwnedFd::from_raw_fd(libc::dup(self.handle(h)?.fd.as_raw_fd())) },
            HKind::File { .. } => return Err(err::INVALID),
            HKind::Dir { .. } => {
                let (dir, _) = self.dir(h, path)?;
                openat2(dir, path, O_PATH, 0)?
            }
        };
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } < 0 {
            return Err(last());
        }
        let kind = match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => 0u8,
            libc::S_IFDIR => 1,
            _ => 2,
        };
        let mut out = [0u8; 17];
        out[0] = kind;
        out[1..9].copy_from_slice(&(st.st_size as u64).to_le_bytes());
        let ms = st.st_mtime * 1000 + st.st_mtime_nsec / 1_000_000;
        out[9..17].copy_from_slice(&ms.to_le_bytes());
        Ok(out)
    }

    /// Entries as repeated `u8 kind, str name`, sorted by name.
    pub fn list(&self, h: i32, path: &str) -> Result<Vec<u8>, i32> {
        let (dir, _) = self.dir(h, path)?;
        let fd = openat2(dir, path, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        let mut items = read_names(fd)?;
        items.sort_by(|a, b| a.1.cmp(&b.1));
        let mut out = Vec::new();
        for (k, n) in items {
            out.push(k);
            out.extend_from_slice(&(n.len() as u32).to_le_bytes());
            out.extend_from_slice(n.as_bytes());
        }
        Ok(out)
    }

    fn parent(&self, h: i32, path: &str) -> Result<(OwnedFd, CString), i32> {
        let (dir, writable) = self.dir(h, path)?;
        if !writable {
            return Err(err::DENIED);
        }
        let (p, name) = split(path);
        if name.is_empty() || name == "." {
            return Err(err::INVALID);
        }
        let pfd = openat2(dir, p, O_PATH | libc::O_DIRECTORY, 0)?;
        Ok((pfd, CString::new(name).map_err(|_| err::INVALID)?))
    }

    pub fn mkdir(&self, h: i32, path: &str) -> i32 {
        match self.parent(h, path) {
            Ok((p, n)) => {
                if unsafe { libc::mkdirat(p.as_raw_fd(), n.as_ptr(), 0o755) } < 0 {
                    last()
                } else {
                    0
                }
            }
            Err(e) => e,
        }
    }

    pub fn remove(&self, h: i32, path: &str) -> i32 {
        match self.parent(h, path) {
            Ok((p, n)) => {
                if unsafe { libc::unlinkat(p.as_raw_fd(), n.as_ptr(), 0) } == 0 {
                    return 0;
                }
                let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if e == libc::EISDIR || e == libc::EPERM {
                    if unsafe { libc::unlinkat(p.as_raw_fd(), n.as_ptr(), libc::AT_REMOVEDIR) } == 0 {
                        return 0;
                    }
                    return last();
                }
                errno_code(e)
            }
            Err(e) => e,
        }
    }

    pub fn rename(&self, h: i32, from: &str, h2: i32, to: &str) -> i32 {
        let (a, b) = match (self.parent(h, from), self.parent(h2, to)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        if unsafe { libc::renameat(a.0.as_raw_fd(), a.1.as_ptr(), b.0.as_raw_fd(), b.1.as_ptr()) } < 0 {
            last()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn sandbox_rules() {
        let base = std::env::temp_dir().join(format!("qa-fs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("data/sub")).unwrap();
        std::fs::write(base.join("secret"), "x").unwrap();
        std::os::unix::fs::symlink("../secret", base.join("data/link")).unwrap();
        let mut fs = Fs::new(Some(open_dir(&base.join("data")).unwrap()), None);
        assert_eq!(fs.open(0, "../secret", READ), err::INVALID);
        assert_eq!(fs.open(0, "/etc/passwd", READ), err::INVALID);
        assert!(fs.open(0, "link", READ) < 0, "a symlink must not escape");
        let fd = fs.open(0, "sub/a.txt", WRITE | CREATE);
        assert!(fd >= 0);
        assert_eq!(fs.write(fd, b"hello"), 5);
        fs.close(fd);
        let fd = fs.open(0, "sub/a.txt", READ);
        assert!(fd >= 0, "reopen: {fd}");
        let mut b = [0u8; 16];
        assert_eq!(fs.read(fd, &mut b), 5);
        assert_eq!(fs.stat(0, "sub/a.txt").unwrap()[0], 0);
        assert!(fs.list(0, "sub").unwrap().len() > 5);
        assert_eq!(fs.rename(0, "sub/a.txt", 0, "b.txt"), 0);
        assert_eq!(fs.remove(0, "b.txt"), 0);
        assert_eq!(fs.mkdir(1, "x"), err::BAD_HANDLE);
        assert_eq!(fs.open(7, "x", READ), err::BAD_HANDLE);
        let _ = std::fs::remove_dir_all(&base);
    }
}
