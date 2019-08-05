use std::io::ErrorKind;
use std::sync::atomic::{self, AtomicBool, Ordering};

const LOCK_TIMEOUT: usize = 64;
static LOCK: AtomicBool = AtomicBool::new(false);

pub(crate) fn lock() -> Result<(), ErrorKind> {
    let mut count = 1;

    loop {
        if let Ok(true) = LOCK.compare_exchange(
            false, true, Ordering::Acquire, Ordering::Relaxed
        ) {
            break;
        }

        if count > LOCK_TIMEOUT {
            return Err(ErrorKind::TimedOut);
        }

        cpu_relax(count);
        count += 1;
    }

    Ok(())
}

#[inline]
pub(crate) fn unlock() {
    LOCK.store(false, Ordering::SeqCst);
}

#[inline(always)]
pub(crate) fn cpu_relax(count: usize) {
    for _ in 0..(1 << count) {
        atomic::spin_loop_hint()
    }
}