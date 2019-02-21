#![allow(dead_code)]

use std::io::ErrorKind;
use std::sync::{atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering, Once, ONCE_INIT};
use std::vec;
use crate::manager::BufferSlice;
use crate::channel::{Sender, Receiver};
use crate::lock::{lock, unlock};

const ONCE: Once = ONCE_INIT;
const DEFAULT_GROWTH: u8 = 4;
const DEFAULT_CAPACITY: usize = 512;

static mut BUFFER: Option<BufferPool> = None;
static mut SIZE_CAP: AtomicUsize = AtomicUsize::new(512);

pub(crate) enum BufOp {
    Reserve(bool),
    Release(usize),
    ReleaseAndExtend(Vec<u8>),
    Extend(usize),
}

pub(crate) enum WorkerOp {
    Cleanup(usize),
    Shutdown,
}

pub(crate) enum SliceStatusQuery {
    Length,
    Capacity,
}

pub(crate) struct BufferPool {
    store: Vec<Vec<u8>>,
    pool: Vec<usize>,
    slice_capacity: usize,
    worker_chan: Sender<WorkerOp>,
    closing: AtomicBool,
}

pub(crate) trait PoolManagement {
    fn make(store: Vec<Vec<u8>>, pool: Vec<usize>, slice_capacity: usize, worker_chan: Sender<WorkerOp>);
    fn default_capacity() -> usize;
    fn slice_stat(id: usize, query: SliceStatusQuery) -> usize;
    fn handle_work(rx: Receiver<WorkerOp>);
    fn manage(command: BufOp) -> Option<BufferSlice>;
    fn reset_and_release(id: usize);
    fn get_writable(id: usize) -> Result<&'static mut Vec<u8>, ErrorKind>;
    fn get_readable(id: usize) -> Result<&'static Vec<u8>, ErrorKind>;
    fn reset_slice(id: usize);
    fn set_size_limit(limit: usize);
}

impl PoolManagement for BufferPool {
    fn make(
        store: Vec<Vec<u8>>,
        pool: Vec<usize>,
        slice_capacity: usize,
        worker_chan: Sender<WorkerOp>
    ) {
        unsafe {
            if store.len() > SIZE_CAP.load(Ordering::SeqCst) {
                SIZE_CAP.store(store.len(), Ordering::SeqCst);
            }

            BUFFER = Some(BufferPool {
                store,
                pool,
                slice_capacity,
                worker_chan,
                closing: AtomicBool::new(false),
            });
        }
    }

    fn default_capacity() -> usize {
        if let Some(buf) = buffer_ref() {
            buf.slice_capacity
        } else {
            // guess the capacity
            DEFAULT_CAPACITY
        }
    }

    fn slice_stat(id: usize, query: SliceStatusQuery) -> usize {
        if let Some(buf) = buffer_ref() {
            match query {
                SliceStatusQuery::Length => buf.store[id].len(),
                SliceStatusQuery::Capacity => buf.store[id].capacity(),
            }
        } else {
            0
        }
    }

    fn handle_work(rx: Receiver<WorkerOp>) {
        for message in rx.recv() {
            match message {
                WorkerOp::Cleanup(id) => BufferPool::manage(BufOp::Release(id)),
                WorkerOp::Shutdown => return,
            };
        }
    }

    fn manage(command: BufOp) -> Option<BufferSlice> {
        if lock().is_err() {
            return None;
        }

        let result = {
            if let Some(buf) = buffer_update() {
                match command {
                    BufOp::Reserve(forced) => buf.reserve(forced),
                    BufOp::Release(id) => {
                        buf.reset(id);
                        buf.release(id);
                        None
                    },
                    BufOp::Extend(count) => {
                        buf.extend(count);
                        None
                    },
                    BufOp::ReleaseAndExtend(vec) => {
                        if buf.store.len() < unsafe { SIZE_CAP.load(Ordering::SeqCst) } {
                            let id = buf.store.len();

                            buf.store.push(vec);
                            buf.pool.push(id);

                            buf.reset(id);
                        }

                        None
                    }
                }
            } else {
                None
            }
        };

        unlock();
        result
    }

    fn reset_and_release(id: usize) {
        if let Some(buf) = buffer_ref() {
            buf.worker_chan
                .send(WorkerOp::Cleanup(id))
                .unwrap_or_else(|err| {
                    eprintln!("Failed to release buffer slice: {}, err: {}", id, err);
                });
        }
    }

    fn get_writable(id: usize) -> Result<&'static mut Vec<u8>, ErrorKind> {
        if let Some(buf) = buffer_update() {
            if buf.closing.load(Ordering::SeqCst) {
                return Err(ErrorKind::NotConnected);
            }

            if id < buf.store.len() {
                return Ok(&mut buf.store[id]);
            } else {
                return Err(ErrorKind::InvalidData);
            }
        }

        Err(ErrorKind::NotConnected)
    }

    fn get_readable(id: usize) -> Result<&'static Vec<u8>, ErrorKind> {
        if let Some(buf) = buffer_update() {
            if buf.closing.load(Ordering::SeqCst) {
                return Err(ErrorKind::NotConnected);
            }

            if id < buf.store.len() {
                return Ok(&buf.store[id]);
            } else {
                return Err(ErrorKind::InvalidData);
            }
        }

        Err(ErrorKind::NotConnected)
    }

    fn reset_slice(id: usize) {
        if let Some(buf) = buffer_update() {
            buf.reset(id);
        }
    }

    fn set_size_limit(limit: usize) {
        unsafe { SIZE_CAP.store(limit, Ordering::SeqCst); }
    }
}

trait PoolOps {
    fn reserve(&mut self, force: bool) -> Option<BufferSlice>;
    fn release(&mut self, id: usize);
    fn reset(&mut self, id: usize);
    fn extend(&mut self, additional: usize) -> usize;
    fn expand_slice(&mut self, id: usize, additional: usize);
}

impl PoolOps for BufferPool {
    fn reserve(&mut self, force: bool) -> Option<BufferSlice> {
        match self.pool.pop() {
            Some(id) => Some(BufferSlice::new(id, None)),
            None => {
                if force {

                    //TODO: try to extend, if failed, use fallback

                    Some(BufferSlice::new(
                        self.extend(DEFAULT_GROWTH as usize), None
                    ))
                } else {
                    None
                }
            }
        }
    }

    fn release(&mut self, id: usize) {
        if id < self.store.len() {
            self.pool.push(id);
        }
    }

    fn reset(&mut self, id: usize) {
        assert!(id < self.store.len());

        let capacity: usize = self.slice_capacity;
        let vec_cap: usize = self.store[id].capacity();

        if vec_cap > capacity {
            self.store[id].truncate(capacity);
        } else if vec_cap < capacity {
            self.store[id].reserve(capacity - vec_cap);
        }

        self.store[id].iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    fn extend(&mut self, additional: usize) -> usize {
        assert!(additional > 0);

        //TODO: do not blow up the roof

        let capacity = self.slice_capacity;
        let start = self.store.len();

        self.store.reserve(additional);
        self.pool.reserve(additional);

        (0..additional).for_each(|offset| {
            self.store.push(vec::from_elem(0, capacity));
            self.pool.push(start + offset);
        });

        // return the last element in the buffer
        self.store.len() - 1
    }

    fn expand_slice(&mut self, id: usize, additional: usize) {
        if id >= self.store.len() {
            return;
        }

        let start = self.store[id].len();
        self.store[id].reserve(additional);

        let end = self.store[id].capacity();
        (start..end).for_each(|_| {
           self.store[id].push(0);
        });
    }
}

impl Drop for BufferPool {
    fn drop(&mut self) {
        *self.closing.get_mut() = true;
    }
}

#[inline]
fn buffer_ref() -> Option<&BufferPool> {
    unsafe { BUFFER.as_ref() }
}

#[inline]
fn buffer_update() -> Option<&mut BufferPool> {
    unsafe { BUFFER.as_mut() }
}