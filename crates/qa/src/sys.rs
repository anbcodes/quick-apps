//! Small differences between Linux and macOS that the rest of the runtime shouldn't see.

use std::os::fd::RawFd;

/// A descriptor that polls readable after `wake` and stops after `drain`. Any thread may wake it.
#[cfg(target_os = "linux")]
pub fn new_wake() -> RawFd {
    unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) }
}

#[cfg(target_os = "linux")]
pub fn wake(fd: RawFd) {
    let one = 1u64;
    unsafe { libc::write(fd, &one as *const u64 as *const _, 8) };
}

#[cfg(target_os = "linux")]
pub fn drain(fd: RawFd) {
    let mut n = 0u64;
    unsafe { libc::read(fd, &mut n as *mut u64 as *mut _, 8) };
}

// macOS has no eventfd, but a kqueue with a user event is one descriptor that can be
// polled, and that any thread can trigger.
#[cfg(target_os = "macos")]
pub fn new_wake() -> RawFd {
    unsafe {
        let kq = libc::kqueue();
        if kq >= 0 {
            libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC);
            let ev = libc::kevent { ident: 1, filter: libc::EVFILT_USER, flags: libc::EV_ADD | libc::EV_CLEAR, fflags: 0, data: 0, udata: std::ptr::null_mut() };
            libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null());
        }
        kq
    }
}

#[cfg(target_os = "macos")]
pub fn wake(fd: RawFd) {
    let ev = libc::kevent { ident: 1, filter: libc::EVFILT_USER, flags: 0, fflags: libc::NOTE_TRIGGER, data: 0, udata: std::ptr::null_mut() };
    unsafe { libc::kevent(fd, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
}

#[cfg(target_os = "macos")]
pub fn drain(fd: RawFd) {
    let mut out: libc::kevent = unsafe { std::mem::zeroed() };
    let zero = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::kevent(fd, std::ptr::null(), 0, &mut out, 1, &zero) };
}

/// Monotonic milliseconds.
pub fn now_ms() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

/// Fills `buf` with random bytes.
pub fn random(buf: &mut [u8]) -> bool {
    let mut off = 0;
    while off < buf.len() {
        #[cfg(target_os = "linux")]
        let n = unsafe { libc::getrandom(buf[off..].as_mut_ptr() as *mut _, buf.len() - off, 0) };
        // getentropy takes at most 256 bytes and returns 0 on success.
        #[cfg(target_os = "macos")]
        let n = {
            let k = (buf.len() - off).min(256);
            if unsafe { libc::getentropy(buf[off..].as_mut_ptr() as *mut _, k) } == 0 {
                k as isize
            } else {
                -1
            }
        };
        if n <= 0 {
            return false;
        }
        off += n as usize;
    }
    true
}

/// Reports changes to folders and the files in them, through a descriptor to poll.
pub struct Watcher {
    fd: RawFd,
    /// kqueue watches files, not folders' contents, so each file and folder is held open.
    #[cfg(target_os = "macos")]
    held: std::cell::RefCell<std::collections::HashMap<std::path::PathBuf, RawFd>>,
}

impl Watcher {
    #[cfg(target_os = "linux")]
    pub fn new() -> Watcher {
        Watcher { fd: unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) } }
    }

    #[cfg(target_os = "macos")]
    pub fn new() -> Watcher {
        let fd = unsafe { libc::kqueue() };
        if fd >= 0 {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        Watcher { fd, held: Default::default() }
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Watches a folder (and `recursive`ly the folders in it). Safe to repeat.
    #[cfg(target_os = "linux")]
    pub fn add(&self, p: &std::path::Path, recursive: bool) {
        let Ok(c) = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()) else { return };
        let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB;
        if unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), mask) } < 0 {
            return;
        }
        self.add_children(p, recursive);
    }

    #[cfg(target_os = "macos")]
    pub fn add(&self, p: &std::path::Path, recursive: bool) {
        if !self.watch_one(p) {
            return;
        }
        // Writes to a file in place don't touch its folder, so the files are watched too.
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_file()) {
                    self.watch_one(&e.path());
                }
            }
        }
        self.add_children(p, recursive);
    }

    fn add_children(&self, p: &std::path::Path, recursive: bool) {
        if recursive {
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    if e.file_type().is_ok_and(|t| t.is_dir()) {
                        self.add(&e.path(), true);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn watch_one(&self, p: &std::path::Path) -> bool {
        if self.held.borrow().contains_key(p) {
            return true;
        }
        let Ok(c) = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()) else { return false };
        // O_EVTONLY: open for notifications only, so the file can still be unmounted or deleted.
        let f = unsafe { libc::open(c.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
        if f < 0 {
            return false;
        }
        let fflags = libc::NOTE_WRITE | libc::NOTE_DELETE | libc::NOTE_EXTEND | libc::NOTE_RENAME | libc::NOTE_ATTRIB;
        let ev = libc::kevent { ident: f as usize, filter: libc::EVFILT_VNODE, flags: libc::EV_ADD | libc::EV_CLEAR, fflags, data: 0, udata: std::ptr::null_mut() };
        unsafe { libc::kevent(self.fd, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        self.held.borrow_mut().insert(p.to_path_buf(), f);
        true
    }

    /// True when something changed since the last call.
    #[cfg(target_os = "linux")]
    pub fn changed(&self) -> bool {
        let mut buf = [0u8; 4096];
        let mut any = false;
        while unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) } > 0 {
            any = true;
        }
        any
    }

    #[cfg(target_os = "macos")]
    pub fn changed(&self) -> bool {
        let mut any = false;
        let zero = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        loop {
            let mut out: libc::kevent = unsafe { std::mem::zeroed() };
            if unsafe { libc::kevent(self.fd, std::ptr::null(), 0, &mut out, 1, &zero) } <= 0 {
                break;
            }
            any = true;
            // A deleted or replaced file's watch is dead; drop it so a new file there is found.
            if out.fflags & (libc::NOTE_DELETE | libc::NOTE_RENAME) != 0 {
                let f = out.ident as RawFd;
                self.held.borrow_mut().retain(|_, v| *v != f);
                unsafe { libc::close(f) };
            }
        }
        any
    }
}
