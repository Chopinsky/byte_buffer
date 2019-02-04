#![allow(dead_code)]

use std::io::ErrorKind;
use std::sync::{Once, ONCE_INIT};
use std::thread;
use std::vec;
use crate::channel::{self as channel};
use crate::buffer::{PoolManagement, BufOp, BufferPool};

const ONCE: Once = ONCE_INIT;

pub struct ByteBuffer {}
impl ByteBuffer {
    pub fn init(size: usize, capacity: usize) {
        ONCE.call_once(|| {
            let mut store = Vec::with_capacity(size);
            let mut pool = Vec::with_capacity(size);

            (0..size).for_each(|id| {
                store.push(vec::from_elem(0, capacity));
                pool.push(id);
            });

            let (sender, receiver) = channel::bounded(8);
            thread::spawn(move || {
                BufferPool::handle_work(receiver);
            });

            BufferPool::make(store, pool, capacity, sender);
        });
    }

    pub fn slice() -> BufferSlice {
        match BufferPool::manage(BufOp::Reserve(true)) {
            Some(val) => val,
            None => BufferSlice {
                id: 0,
                fallback: Some(vec::from_elem(0, BufferPool::capacity())),
            },
        }
    }

    #[inline]
    pub fn try_slice() -> Option<BufferSlice> {
        BufferPool::manage(BufOp::Reserve(false))
    }
}

pub struct BufferSlice {
    id: usize,
    fallback: Option<Vec<u8>>,
}

impl BufferSlice {
    pub(crate) fn new(id: usize, fallback: Option<Vec<u8>>) -> Self {
        BufferSlice {id, fallback}
    }

    pub fn as_writable(&mut self) -> Result<&mut [u8], ErrorKind> {
        match self.fallback {
            Some(ref mut vec) => return Ok(vec.as_mut_slice()),
            None => {},
        }

        match BufferPool::get_writable(self.id) {
            Ok(vec) => Ok(vec.as_mut_slice()),
            Err(e) => Err(e),
        }
    }

    pub fn as_writable_vec(&mut self) -> Result<&mut Vec<u8>, ErrorKind> {
        match self.fallback {
            Some(ref mut vec) => return Ok(vec),
            None => {},
        }

        BufferPool::get_writable(self.id)
    }

    pub fn as_readable(&self) -> Result<&[u8], ErrorKind> {
        match self.fallback {
            Some(ref vec) => return Ok(vec.as_slice()),
            None => {},
        }

        match BufferPool::get_readable(self.id) {
            Ok(vec) => Ok(vec.as_slice()),
            Err(e) => Err(e),
        }
    }

    pub fn as_readable_vec(&self) -> Result<&Vec<u8>, ErrorKind> {
        match self.fallback {
            Some(ref vec) => return Ok(vec),
            None => {},
        }

        BufferPool::get_readable(self.id)
    }

    pub fn copy_to_vec(&self) -> Result<Vec<u8>, ErrorKind> {
        // this will hard-copy the vec content
        Ok(self.as_readable()?.to_vec())
    }
}

impl Drop for BufferSlice {
    fn drop(&mut self) {
        if self.id == 0 && self.fallback.is_some() {
            BufferPool::manage(BufOp::ReleaseAndExtend(self.fallback.take().unwrap()));
        } else {
            BufferPool::reset_and_release(self.id);
        }
    }
}
