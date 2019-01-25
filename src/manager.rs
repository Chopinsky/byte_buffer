use std::io::ErrorKind;
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Once, ONCE_INIT};
use std::time::{Duration, SystemTime};
use crate::buffer::{ByteBuffer, BufferOp};

static mut BUFFER: Option<Vec<ByteBuffer>> = None;
static mut DEFAULT_CAPACITY: usize = 1;

static mut LOCK: AtomicBool = AtomicBool::new(false);
static mut BUF_SIZE: AtomicUsize = AtomicUsize::new(0);

const ONCE: Once = ONCE_INIT;
const LOCK_TIMEOUT: Duration = Duration::from_millis(64);
const DEFAULT_GROWTH: usize = 4;
const BUF_ROOF: usize = 65535;

pub fn init(size: usize, capacity: usize) {
    ONCE.call_once(|| {
        let mut buffer = Vec::with_capacity(size);

        (0..size).for_each(|_| {
            buffer.push(ByteBuffer::new(capacity));
        });

        unsafe {
            BUFFER = Some(buffer);
            DEFAULT_CAPACITY = capacity;
            BUF_SIZE.fetch_add(size, Ordering::SeqCst);
        }
    });
}

pub fn reserve() -> ByteBuffer {
    let buf = match try_reserve() {
        Some(buf) => buf,
        None => unsafe {
            let cap = DEFAULT_CAPACITY;
            let (mut buf, inc) =
                if let Some(ref mut buffer) = BUFFER {
                    // the BUFFER store is still valid
                    if BUF_SIZE.load(Ordering::SeqCst) > BUF_ROOF {
                        // already blow the memory guard, be gentle
                        (ByteBuffer::new(cap), 1)
                    } else {
                        // grow the buffer with pre-determined size
                        if lock().is_ok() {
                            (0..DEFAULT_GROWTH - 1).for_each(|_| {
                                buffer.push(ByteBuffer::new(cap));
                            });

                            unlock();
                        }

                        // don't bother pop again, lend a new slice
                        (ByteBuffer::new(cap), DEFAULT_GROWTH)
                    }
                } else {
                    // can't get a hold of the BUFFER store, just make the slice
                    (ByteBuffer::new(cap), 1)
                };

            // update the buffer size -- including the lent out ones
            BUF_SIZE.fetch_add(inc, Ordering::SeqCst);

            buf.update_status(true);
            buf
        }
    };

    buf
}

pub fn try_reserve() -> Option<ByteBuffer> {
    unsafe {
        // wait for the lock
        if lock().is_err() {
            return None;
        }

        let mut buf =
            if let Some(ref mut buffer) = BUFFER {
                buffer.pop()
            } else {
                None
            };

        // the protected section is finished, release the lock
        unlock();

        if let Some(ref mut b) = buf {
            b.update_status(true);
        }

        buf
    }
}

pub fn release(buf: ByteBuffer) {
    push_back(buf);
}

pub(crate) fn push_back(buf: ByteBuffer) {
    let mut buf_slice = buf;

    // the ownership of the buffer slice is returned, update the status as so regardless if it
    // needs to be dropped right away
    buf_slice.update_status(false);

    unsafe {
        if BUF_SIZE.load(Ordering::SeqCst) > BUF_ROOF {
            // if we've issued too many buffer slices, just let this one expire on itself
            BUF_SIZE.fetch_sub(1, Ordering::SeqCst);

            return;
        }

        if let Some(ref mut buffer) = BUFFER {
            buf_slice.reset();

            if lock().is_ok() {
                buffer.push(buf_slice);
                unlock();
            }
        }
    }
}

pub(crate) fn buffer_capacity() -> usize {
    unsafe { DEFAULT_CAPACITY }
}

fn lock() -> Result<(), ErrorKind> {
    let start = SystemTime::now();
    loop {
        let locked = unsafe {
            match LOCK.compare_exchange(
                false, true, Ordering::SeqCst, Ordering::SeqCst
            ) {
                Ok(res) => res == false,
                Err(_) => false,
            }
        };

        if locked {
            break;
        }

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

fn unlock() {
    unsafe {
        *LOCK.get_mut() = false;
    }
}
