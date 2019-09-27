#![allow(dead_code)]

use crate::channel::{Receiver, Sender};
use crate::lock::{lock, unlock};
use crate::utils::*;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::vec;

const DEFAULT_GROWTH: usize = 4;
const DEFAULT_CAPACITY: usize = 512;

static mut BUFFER: Option<BufferPool> = None;
static mut SIZE_CAP: AtomicUsize = AtomicUsize::new(512);

struct Store {
    buf: Vec<u8>,
    taken: AtomicBool,
}

pub(crate) struct BufferPool {
    store: Vec<Vec<u8>>,
    //    pool: Vec<AtomicU8>,
    slice_capacity: usize,
    worker_chan: Sender<WorkerOp>,
    closing: AtomicBool,
    barrier: AtomicBool,
    visitors: AtomicUsize,
}

pub(crate) trait PoolManagement {
    fn make(
        store: Vec<Vec<u8>>,
        //        pool: Vec<usize>,
        slice_capacity: usize,
        worker_chan: Sender<WorkerOp>,
    );
    fn default_capacity() -> usize;
    fn slice_stat(id: usize, query: SliceStatusQuery) -> usize;
    fn handle_work(rx: Receiver<WorkerOp>);
    fn exec(command: BufOp) -> Option<usize>;
    fn reset_and_release(id: usize, dirty: bool);
    fn get_writable(id: usize) -> Result<&'static mut Vec<u8>, ErrorKind>;
    fn get_readable(id: usize) -> Result<&'static Vec<u8>, ErrorKind>;
    fn reset_slice(id: usize);
    fn set_size_limit(limit: usize);
}

impl PoolManagement for BufferPool {
    fn make(
        store: Vec<Vec<u8>>,
        //        pool: Vec<usize>,
        slice_capacity: usize,
        worker_chan: Sender<WorkerOp>,
    ) {
        unsafe {
            if store.len() > SIZE_CAP.load(Ordering::SeqCst) {
                SIZE_CAP.store(store.len(), Ordering::SeqCst);
            }

            BUFFER.replace(BufferPool {
                store,
                //                pool,
                slice_capacity,
                worker_chan,
                closing: AtomicBool::new(false),
                barrier: AtomicBool::new(false),
                visitors: AtomicUsize::new(0),
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
        loop {
            match rx.recv() {
                Ok(message) => {
                    match message {
                        WorkerOp::Cleanup(id, dirty) => BufferPool::exec(BufOp::Release(id, dirty)),
                        WorkerOp::Shutdown => return,
                    };
                }
                Err(_) => return,
            };
        }
    }

    fn exec(command: BufOp) -> Option<usize> {
        if lock().is_err() {
            return None;
        }

        let mut result: Option<usize> = None;
        if let Some(buf) = buffer_mut() {
            match command {
                BufOp::Reserve(forced) => {
                    if let Some(id) = buf.try_reserve() {
                        result = Some(id)
                    } else if forced {
                        //TODO: try extend, and if failed, generate fallback
                        result = Some(buf.extend(DEFAULT_GROWTH));
                    }
                }
                BufOp::Release(id, dirty) => {
                    buf.release(id);

                    if dirty {
                        buf.reset(id);
                    }
                }
                BufOp::Extend(count) => {
                    //TODO: try extend, and if failed, fallback to None
                    result = Some(buf.extend(count));
                }
                BufOp::ReleaseAndExtend(vec, dirty) => {
                    if buf.store.len() < unsafe { SIZE_CAP.load(Ordering::SeqCst) } {
                        let id = buf.store.len();

                        buf.store.push(vec);
                        //                        buf.pool.push(id);

                        if dirty {
                            buf.reset(id);
                        }
                    }
                }
            }
        }

        unlock();
        result
    }

    fn reset_and_release(id: usize, dirty: bool) {
        if let Some(buf) = buffer_ref() {
            buf.worker_chan
                .send(WorkerOp::Cleanup(id, dirty))
                .unwrap_or_else(|err| {
                    eprintln!("Failed to release buffer slice: {}, err: {}", id, err);
                });
        }
    }

    fn get_writable(id: usize) -> Result<&'static mut Vec<u8>, ErrorKind> {
        if let Some(buf) = buffer_mut() {
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
        if let Some(buf) = buffer_ref() {
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
        if let Some(buf) = buffer_mut() {
            buf.reset(id);
        }
    }

    fn set_size_limit(limit: usize) {
        unsafe {
            SIZE_CAP.store(limit, Ordering::SeqCst);
        }
    }
}

trait PoolOps {
    fn try_reserve(&mut self) -> Option<usize>;
    fn release(&mut self, id: usize);
    fn reset(&mut self, id: usize);
    fn extend(&mut self, additional: usize) -> usize;
    fn expand_slice(&mut self, id: usize, additional: usize);
}

impl PoolOps for BufferPool {
    #[inline]
    fn try_reserve(&mut self) -> Option<usize> {
        //        self.pool.pop()
        None
    }

    fn release(&mut self, id: usize) {
        if id < self.store.len() {
            //            self.pool.push(id);
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
        //        self.pool.reserve(additional);

        (0..additional).for_each(|offset| {
            self.store.push(vec::from_elem(0, capacity));
            //            self.pool.push(start + offset);
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

        self.worker_chan
            .send(WorkerOp::Shutdown)
            .unwrap_or_else(|err| {
                eprintln!("Failed to close the worker thread, error code: {}", err);
            });
    }
}

#[inline]
fn buffer_ref() -> Option<&'static BufferPool> {
    unsafe { BUFFER.as_ref() }
}

#[inline]
fn buffer_mut() -> Option<&'static mut BufferPool> {
    unsafe { BUFFER.as_mut() }
}
