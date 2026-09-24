//! Background-thread spawning with an in-flight counter, so the status bar can
//! show a busy indicator while DB work is running.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static PENDING: AtomicUsize = AtomicUsize::new(0);
/// When the counter last went from idle to busy; drives the show-delay.
static BUSY_SINCE: Mutex<Option<Instant>> = Mutex::new(None);

/// Decrements `PENDING` on drop, so a panicking job still clears itself.
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Drop-in for `std::thread::spawn` that counts the job as in-flight until it returns.
/// The closure's return value is discarded.
pub fn spawn<F, T>(f: F)
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    PENDING.fetch_add(1, Ordering::SeqCst);
    let guard = Guard;
    std::thread::spawn(move || {
        let _guard = guard;
        f()
    });
}

/// True once background work has been in flight for at least `delay`. The delay
/// keeps sub-frame DB ops from flashing the indicator.
pub fn busy(delay: Duration) -> bool {
    let mut since = BUSY_SINCE.lock().unwrap_or_else(|e| e.into_inner());
    if PENDING.load(Ordering::SeqCst) == 0 {
        *since = None;
        return false;
    }
    since.get_or_insert_with(Instant::now).elapsed() >= delay
}
