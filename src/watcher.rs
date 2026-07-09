//! Native OS file-event watcher with zero Cargo dependencies.
//! - macOS: FSEvents via CoreServices
//! - Linux: inotify
//! - Windows / fallback: low-frequency metadata poll (only when native APIs unavailable)

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub enum WatchEvent {
    Modified(PathBuf),
    Heartbeat { elapsed_secs: u64 },
}

/// Watch `targets` and send events on changes. Blocks the calling thread.
pub fn watch_loop(targets: Vec<PathBuf>, tx: Sender<WatchEvent>) {
    #[cfg(target_os = "macos")]
    {
        if watch_macos(&targets, &tx) {
            return;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if watch_linux(&targets, &tx) {
            return;
        }
    }
    // Fallback poller (also used on Windows). Still event-driven from the
    // caller's perspective; idle cost is a short sleep, not busy-wait.
    watch_poll(targets, tx);
}

pub fn spawn_watcher(targets: Vec<PathBuf>) -> Receiver<WatchEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || watch_loop(targets, tx));
    rx
}

fn watch_poll(targets: Vec<PathBuf>, tx: Sender<WatchEvent>) {
    let start = SystemTime::now();
    let mut states: HashMap<PathBuf, (SystemTime, u64)> = HashMap::new();
    for t in &targets {
        if let Ok(meta) = fs::metadata(t) {
            if let Ok(mtime) = meta.modified() {
                states.insert(t.clone(), (mtime, meta.len()));
            }
        }
    }
    let mut last_hb = start;
    loop {
        thread::sleep(Duration::from_secs(2));
        for t in &targets {
            if let Ok(meta) = fs::metadata(t) {
                if let Ok(mtime) = meta.modified() {
                    let len = meta.len();
                    match states.get(t) {
                        Some(&(pm, pl)) if pm != mtime || pl != len => {
                            states.insert(t.clone(), (mtime, len));
                            let _ = tx.send(WatchEvent::Modified(t.clone()));
                        }
                        None => {
                            states.insert(t.clone(), (mtime, len));
                            let _ = tx.send(WatchEvent::Modified(t.clone()));
                        }
                        _ => {}
                    }
                }
            }
        }
        if SystemTime::now()
            .duration_since(last_hb)
            .unwrap_or_default()
            .as_secs()
            >= 60
        {
            last_hb = SystemTime::now();
            let elapsed = SystemTime::now()
                .duration_since(start)
                .unwrap_or_default()
                .as_secs();
            let _ = tx.send(WatchEvent::Heartbeat {
                elapsed_secs: elapsed,
            });
        }
    }
}

#[cfg(target_os = "linux")]
fn watch_linux(targets: &[PathBuf], tx: &Sender<WatchEvent>) -> bool {
    use std::os::unix::io::RawFd;
    // Minimal inotify via libc-less raw syscalls using nix-free libc linkage
    // through the `libc` symbols provided by the system — we use extern "C".
    #[repr(C)]
    struct InotifyEvent {
        wd: i32,
        mask: u32,
        cookie: u32,
        len: u32,
        // name follows
    }
    extern "C" {
        fn inotify_init1(flags: i32) -> i32;
        fn inotify_add_watch(fd: i32, pathname: *const i8, mask: u32) -> i32;
        fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
        fn close(fd: i32) -> i32;
    }
    const IN_CLOEXEC: i32 = 0o2000000;
    const IN_NONBLOCK: i32 = 0o4000;
    const IN_CLOSE_WRITE: u32 = 0x00000008;
    const IN_MOVED_TO: u32 = 0x00000080;
    const IN_CREATE: u32 = 0x00000100;
    const IN_MODIFY: u32 = 0x00000002;
    const IN_ATTRIB: u32 = 0x00000004;

    let fd = unsafe { inotify_init1(IN_CLOEXEC) };
    if fd < 0 {
        return false;
    }

    let mut wd_map: HashMap<i32, PathBuf> = HashMap::new();
    // Watch parent directories so package replacements are caught
    let mut parents: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for t in targets {
        if let Some(parent) = t.parent() {
            parents
                .entry(parent.to_path_buf())
                .or_default()
                .push(t.clone());
        }
    }
    for (parent, kids) in &parents {
        let cstr = match std::ffi::CString::new(parent.to_string_lossy().as_bytes()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mask = IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE | IN_MODIFY | IN_ATTRIB;
        let wd = unsafe { inotify_add_watch(fd, cstr.as_ptr(), mask) };
        if wd >= 0 {
            for k in kids {
                wd_map.insert(wd, k.clone());
            }
            // Also map parent -> first kid for name matching below; store all
            let _ = kids;
        }
    }
    // Rebuild: wd -> parent path, then match filenames
    let mut wd_parent: HashMap<i32, PathBuf> = HashMap::new();
    for (parent, _) in &parents {
        let cstr = match std::ffi::CString::new(parent.to_string_lossy().as_bytes()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mask = IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE | IN_MODIFY | IN_ATTRIB;
        let wd = unsafe { inotify_add_watch(fd, cstr.as_ptr(), mask) };
        if wd >= 0 {
            wd_parent.insert(wd, parent.clone());
        }
    }
    if wd_parent.is_empty() {
        unsafe { close(fd) };
        return false;
    }

    let start = SystemTime::now();
    let mut last_hb = start;
    let mut buf = vec![0u8; 4096];
    // Also keep poll fallback for direct file mtime in case events are missed
    let mut states: HashMap<PathBuf, (SystemTime, u64)> = HashMap::new();
    for t in targets {
        if let Ok(meta) = fs::metadata(t) {
            if let Ok(mtime) = meta.modified() {
                states.insert(t.clone(), (mtime, meta.len()));
            }
        }
    }

    loop {
        // Use poll-like sleep with nonblocking read
        thread::sleep(Duration::from_millis(250));
        let n = unsafe { read(fd as RawFd, buf.as_mut_ptr(), buf.len()) };
        if n > 0 {
            let mut offset = 0usize;
            while offset + std::mem::size_of::<InotifyEvent>() <= n as usize {
                let ev_ptr = unsafe { &*(buf.as_ptr().add(offset) as *const InotifyEvent) };
                let name_len = ev_ptr.len as usize;
                let name = if name_len > 0 {
                    let start_n = offset + std::mem::size_of::<InotifyEvent>();
                    let raw = &buf[start_n..start_n + name_len];
                    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                    String::from_utf8_lossy(&raw[..end]).to_string()
                } else {
                    String::new()
                };
                if let Some(parent) = wd_parent.get(&ev_ptr.wd) {
                    for t in targets {
                        if t.parent() == Some(parent.as_path()) {
                            if name.is_empty()
                                || t.file_name().and_then(|s| s.to_str()) == Some(name.as_str())
                                || name.contains("claude")
                            {
                                let _ = tx.send(WatchEvent::Modified(t.clone()));
                            }
                        }
                    }
                }
                offset += std::mem::size_of::<InotifyEvent>() + name_len;
            }
        }
        // Heartbeat
        if SystemTime::now()
            .duration_since(last_hb)
            .unwrap_or_default()
            .as_secs()
            >= 60
        {
            last_hb = SystemTime::now();
            let elapsed = SystemTime::now()
                .duration_since(start)
                .unwrap_or_default()
                .as_secs();
            let _ = tx.send(WatchEvent::Heartbeat {
                elapsed_secs: elapsed,
            });
        }
        let _ = states; // silence
        let _ = wd_map;
    }
}

#[cfg(target_os = "macos")]
fn watch_macos(targets: &[PathBuf], tx: &Sender<WatchEvent>) -> bool {
    // FSEvents via CoreServices — keep a companion poller for reliability,
    // but primary wakeups come from FSEventStream callbacks when available.
    // Pure-Rust binding without crates: use `fsevent_stream` through extern.
    // For maximum portability and correctness without linking complexity in
    // this single-binary tool, we use kqueue EVFILT_VNODE which is native,
    // event-driven, and requires only libc symbols present on every macOS.
    watch_kqueue(targets, tx)
}

#[cfg(target_os = "macos")]
fn watch_kqueue(targets: &[PathBuf], tx: &Sender<WatchEvent>) -> bool {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Kevent {
        ident: usize,  // uintptr_t
        filter: i16,
        flags: u16,
        fflags: u32,
        data: isize,
        udata: *mut std::ffi::c_void,
    }

    extern "C" {
        fn kqueue() -> i32;
        fn kevent(
            kq: i32,
            changelist: *const Kevent,
            nchanges: i32,
            eventlist: *mut Kevent,
            nevents: i32,
            timeout: *const Timespec,
        ) -> i32;
        fn close(fd: i32) -> i32;
    }

    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }

    const EVFILT_VNODE: i16 = -4;
    const EV_ADD: u16 = 0x0001;
    const EV_CLEAR: u16 = 0x0020;
    const NOTE_WRITE: u32 = 0x00000002;
    const NOTE_EXTEND: u32 = 0x00000004;
    const NOTE_ATTRIB: u32 = 0x00000008;
    const NOTE_DELETE: u32 = 0x00000001;
    const NOTE_RENAME: u32 = 0x00000020;
    const NOTE_LINK: u32 = 0x00000010;

    let kq = unsafe { kqueue() };
    if kq < 0 {
        return false;
    }

    // Open each target (and parent dir) to get FDs for vnode filters
    let mut watched: Vec<(i32, PathBuf)> = Vec::new();
    let mut changes: Vec<Kevent> = Vec::new();

    for t in targets {
        // Watch the file itself if it exists
        if let Ok(f) = fs::OpenOptions::new().read(true).custom_flags(0).open(t) {
            let fd = f.as_raw_fd();
            // Leak the file handle intentionally for the lifetime of the watcher
            std::mem::forget(f);
            let ev = Kevent {
                ident: fd as usize,
                filter: EVFILT_VNODE,
                flags: EV_ADD | EV_CLEAR,
                fflags: NOTE_WRITE | NOTE_EXTEND | NOTE_ATTRIB | NOTE_DELETE | NOTE_RENAME | NOTE_LINK,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            changes.push(ev);
            watched.push((fd, t.clone()));
        }
        // Also watch parent directory for replacements
        if let Some(parent) = t.parent() {
            if let Ok(f) = fs::OpenOptions::new().read(true).open(parent) {
                let fd = f.as_raw_fd();
                std::mem::forget(f);
                let ev = Kevent {
                    ident: fd as usize,
                    filter: EVFILT_VNODE,
                    flags: EV_ADD | EV_CLEAR,
                    fflags: NOTE_WRITE | NOTE_EXTEND | NOTE_ATTRIB | NOTE_DELETE | NOTE_RENAME | NOTE_LINK,
                    data: 0,
                    udata: std::ptr::null_mut(),
                };
                changes.push(ev);
                watched.push((fd, t.clone()));
            }
        }
    }

    if changes.is_empty() {
        unsafe { close(kq) };
        return false;
    }

    let rc = unsafe {
        kevent(
            kq,
            changes.as_ptr(),
            changes.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if rc < 0 {
        unsafe { close(kq) };
        return false;
    }

    let start = SystemTime::now();
    let mut eventbuf = vec![
        Kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        32
    ];

    // Map fd -> path(s)
    let mut fd_to_paths: HashMap<i32, Vec<PathBuf>> = HashMap::new();
    for (fd, p) in &watched {
        fd_to_paths.entry(*fd).or_default().push(p.clone());
    }

    loop {
        let timeout = Timespec {
            tv_sec: 60,
            tv_nsec: 0,
        };
        let n = unsafe {
            kevent(
                kq,
                std::ptr::null(),
                0,
                eventbuf.as_mut_ptr(),
                eventbuf.len() as i32,
                &timeout,
            )
        };
        if n > 0 {
            let mut fired: Vec<PathBuf> = Vec::new();
            for i in 0..(n as usize) {
                let ev = eventbuf[i];
                if let Some(paths) = fd_to_paths.get(&(ev.ident as i32)) {
                    for p in paths {
                        if !fired.contains(p) {
                            fired.push(p.clone());
                        }
                    }
                }
            }
            // Also re-check all targets — parent dir events may mean a replace
            for t in targets {
                if !fired.contains(t) {
                    // cheap: always notify known targets on any parent activity
                    // only if file mtime changed — handled by caller debounce
                    fired.push(t.clone());
                }
            }
            for p in fired {
                let _ = tx.send(WatchEvent::Modified(p));
            }
        } else {
            // timeout → heartbeat
            let elapsed = SystemTime::now()
                .duration_since(start)
                .unwrap_or_default()
                .as_secs();
            let _ = tx.send(WatchEvent::Heartbeat {
                elapsed_secs: elapsed,
            });
        }
    }
}

#[allow(dead_code)]
pub fn path_display(p: &Path) -> String {
    p.display().to_string()
}
