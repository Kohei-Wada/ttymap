//! Thread-local panic hook silencing.
//!
//! [`silence_panics`] runs a closure under [`catch_unwind`], suppressing the
//! panic hook output **only on the current thread**. Panics on other threads
//! continue to print normally. This replaces the fragile pattern of
//! temporarily swapping the process-wide panic hook, which would silence any
//! concurrent panic from unrelated threads during the window.
//!
//! A single hook is installed lazily on first call via [`std::sync::Once`].

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;
use std::thread;

thread_local! {
    static SILENT: Cell<bool> = const { Cell::new(false) };
}

static INSTALLED: Once = Once::new();

fn ensure_hook_installed() {
    INSTALLED.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if SILENT.with(|s| s.get()) {
                return;
            }
            default(info);
        }));
    });
}

/// Run `f` with panic hook output suppressed on the current thread only.
/// Returns the result of [`catch_unwind`] (`Ok(value)` or `Err(payload)`).
pub fn silence_panics<F, R>(f: F) -> thread::Result<R>
where
    F: FnOnce() -> R,
{
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            SILENT.with(|s| s.set(false));
        }
    }

    ensure_hook_installed();
    SILENT.with(|s| s.set(true));
    let _guard = Guard;
    panic::catch_unwind(AssertUnwindSafe(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_ok_when_closure_does_not_panic() {
        let result = silence_panics(|| 42);
        assert_eq!(result.ok(), Some(42));
    }

    #[test]
    fn catches_panic_and_returns_err() {
        let result = silence_panics(|| {
            panic!("intentional");
        });
        assert!(result.is_err());
    }

    #[test]
    fn flag_is_reset_after_panic() {
        let _ = silence_panics(|| panic!("boom"));
        assert!(!SILENT.with(|s| s.get()));
    }

    #[test]
    fn flag_is_reset_after_normal_return() {
        let _ = silence_panics(|| 1);
        assert!(!SILENT.with(|s| s.get()));
    }

    #[test]
    fn other_thread_panic_is_not_silenced() {
        use std::sync::{Arc, Mutex};

        ensure_hook_installed();

        let seen = Arc::new(Mutex::new(false));
        let seen_clone = seen.clone();

        // Install a probe hook on top of the silencing one. The silencing hook
        // returns early only when *our* thread's SILENT flag is set, so a
        // panic from another thread should still reach this probe.
        let existing = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            *seen_clone.lock().unwrap() = true;
            existing(info);
        }));

        // Silence this thread, then panic from a different thread.
        let _ = silence_panics(|| {
            let h = thread::spawn(|| panic!("from other thread"));
            let _ = h.join(); // swallow the JoinError
        });

        assert!(
            *seen.lock().unwrap(),
            "other-thread panic should reach the hook even while this thread is silenced"
        );
    }
}
