use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Once, ONCE_INIT};
use std::vec;
use crate::buffer::ByteBuffer;

static mut BUFFER: Option<Vec<Vec<u8>>> = None;
static mut DEFAULT_CAPACITY: usize = 1;

static mut LOCK: AtomicBool = AtomicBool::new(false);
static mut BUF_SIZE: AtomicUsize = AtomicUsize::new(0);

const ONCE: Once = ONCE_INIT;
const DEFAULT_GROWTH: usize = 4;
const BUF_ROOF: usize = 65535;

pub fn init(size: usize, capacity: usize) {
    ONCE.call_once(|| {
        let mut buffer = Vec::with_capacity(size);

        (0..size).for_each(|_| {
            buffer.push(vec::from_elem(0, capacity));
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
            let (vec, inc) =
                if let Some(ref mut buffer) = BUFFER {
                    // the BUFFER store is still valid
                    if BUF_SIZE.load(Ordering::SeqCst) > BUF_ROOF {
                        // already blow the memory guard, be gentle
                        (vec::from_elem(0, cap), 1)
                    } else {
                        // grow the buffer with pre-determined size
                        (0..DEFAULT_GROWTH).for_each(|_| {
                            buffer.push(vec::from_elem(0, cap));
                        });

                        // don't bother pop again, lend a new slice
                        (vec::from_elem(0, cap), DEFAULT_GROWTH + 1)
                    }
                } else {
                    // can't get a hold of the BUFFER store, just make the slice
                    (vec::from_elem(0, cap), 1)
                };

            // update the buffer size -- including the lent out ones
            BUF_SIZE.fetch_add(inc, Ordering::SeqCst);

            ByteBuffer::new(vec)
        }
    };

    buf
}

pub fn try_reserve() -> Option<ByteBuffer> {
    unsafe {
        // wait for the lock
        loop {
            // use a loop-and-hold method for cheap lock check
            if lock() { break; }
        }

        let res =
            if let Some(ref mut buffer) = BUFFER {
                match buffer.pop() {
                    Some(vec) => Some(ByteBuffer::new(vec)),
                    None => None,
                }
            } else {
                None
            };

        // the protected section is finished, release the lock
        unlock();

        res
    }
}

pub(crate) fn push_back(vec: Vec<u8>) {
    unsafe {
        if BUF_SIZE.load(Ordering::SeqCst) > BUF_ROOF {
            // if we've issued too many buffer slices, just let this one expire on itself
            BUF_SIZE.fetch_sub(1, Ordering::SeqCst);
            return;
        }

        if let Some(ref mut buffer) = BUFFER {
            buffer.push(vec);
        }
    }
}

pub(crate) fn buffer_capacity() -> usize {
    unsafe { DEFAULT_CAPACITY }
}

fn lock() -> bool {
    unsafe {
        match LOCK.compare_exchange(
            false, true, Ordering::SeqCst, Ordering::SeqCst
        ) {
            Ok(res) => res == false,
            Err(_) => false,
        }
    }
}

fn unlock() {
    unsafe {
        *LOCK.get_mut() = false;
    }
}
