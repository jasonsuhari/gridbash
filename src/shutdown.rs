use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static TERMINAL_LOST: AtomicBool = AtomicBool::new(false);

/// Turns SIGTERM and SIGHUP into a request the event loop can act on, so a
/// restart or a closed terminal tab saves the session and restores the
/// terminal instead of dying mid-frame.
pub fn listen() {
    #[cfg(unix)]
    {
        extern "C" fn request(_: libc::c_int) {
            REQUESTED.store(true, Ordering::SeqCst);
        }
        let handler = request as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe {
            libc::signal(libc::SIGTERM, handler);
            libc::signal(libc::SIGHUP, handler);
        }
    }
}

/// The terminal closed under GridBash, so there is nothing left to restore.
pub fn terminal_lost() {
    TERMINAL_LOST.store(true, Ordering::SeqCst);
    REQUESTED.store(true, Ordering::SeqCst);
}

pub fn was_terminal_lost() -> bool {
    TERMINAL_LOST.load(Ordering::SeqCst)
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}
