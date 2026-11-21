//! SIGINT and SIGTERM set a flag the server loop reads. The standard library has no signal API,
//! so this declares the C library's `signal` (already linked on Unix). The handler only stores
//! to an atomic, which is async-signal-safe. Elsewhere (Windows) nothing is installed.

use std::sync::atomic::AtomicBool;

/// Set when the process was asked to stop.
pub static STOP: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
#[allow(
    unsafe_code,
    reason = "the one call into the C library for signals; std has none"
)]
mod unix {
    use super::STOP;
    use std::sync::atomic::Ordering;

    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }

    extern "C" fn on_signal(_: i32) {
        STOP.store(true, Ordering::SeqCst);
    }

    pub fn install() {
        // SIGINT is 2 and SIGTERM is 15 on Linux and macOS.
        for sig in [2, 15] {
            // SAFETY: `signal` is given a valid handler that only stores to an atomic.
            unsafe {
                signal(sig, on_signal);
            }
        }
    }
}

/// Installs the handlers.
pub fn install() {
    #[cfg(unix)]
    unix::install();
}
