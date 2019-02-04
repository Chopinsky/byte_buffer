use std::io::ErrorKind;
use std::sync::{atomic::AtomicBool, atomic::Ordering};
use std::time::{Duration, SystemTime};

const LOCK_TIMEOUT: Duration = Duration::from_millis(64);
static mut LOCK: AtomicBool = AtomicBool::new(false);

pub(crate) fn lock() -> Result<(), ErrorKind> {
    let start = SystemTime::now();

    loop {
        unsafe {
            match LOCK.compare_exchange(
                false, true, Ordering::SeqCst, Ordering::SeqCst
            ) {
                Ok(res) => if res == false {
                    // if not locked previously, we've grabbed the lock and break the wait
                    break;
                },
                Err(_) => {
                    // locked by someone else,
                },
            }
        };

        match start.elapsed() {
            Ok(period) => {
                if period > LOCK_TIMEOUT {
                    return Err(ErrorKind::TimedOut);
                }
            },
            _ => return Err(ErrorKind::TimedOut),
        }
    }

    Ok(())
}

#[inline]
pub(crate) fn unlock() {
    unsafe { *LOCK.get_mut() = false; }
}