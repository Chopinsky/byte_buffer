#![allow(dead_code)]

use std::io::ErrorKind;
use std::str;
use std::sync::{Once, ONCE_INIT};
use std::thread;
use std::vec;
use crate::channel::{self as channel};
use crate::buffer::{PoolManagement, BufOp, BufferPool, SliceStatusQuery};

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
                fallback: Some(vec::from_elem(0, BufferPool::default_capacity())),
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

    pub fn as_writable(&mut self) -> &mut [u8] {
        if let Some(ref mut vec) = self.fallback {
            return vec.as_mut_slice();
        }

        match BufferPool::get_writable(self.id) {
            Ok(vec) => vec.as_mut_slice(),
            Err(_) => {
                self.fallback = Some(vec::from_elem(0, BufferPool::default_capacity()));
                if let Some(ref mut vec) = self.fallback {
                    return vec.as_mut_slice();
                }

                unreachable!();
            },
        }
    }

    pub fn as_writable_vec(&mut self) -> &mut Vec<u8> {
        if let Some(ref mut vec) = self.fallback {
            return vec;
        }

        match BufferPool::get_writable(self.id) {
            Ok(vec) => vec,
            Err(_) => {
                self.fallback = Some(vec::from_elem(0, BufferPool::default_capacity()));
                if let Some(ref mut vec) = self.fallback {
                    return vec;
                }

                unreachable!();
            },
        }
    }

    pub fn read(&self) -> Option<&[u8]> {
        if let Some(ref vec) = self.fallback {
            return Some(vec.as_slice());
        }

        match BufferPool::get_readable(self.id) {
            Ok(vec) => Some(vec.as_slice()),
            Err(e) => {
                eprintln!("Failed to read the buffer: {:?}...", e);
                None
            },
        }
    }

    pub fn read_as_vec(&self) -> Option<&Vec<u8>> {
        if let Some(ref vec) = self.fallback {
            return Some(vec);
        }

        match BufferPool::get_readable(self.id) {
            Ok(vec) => Some(vec),
            Err(e) => {
                eprintln!("Failed to read the buffer: {:?}...", e);
                None
            },
        }
    }

    pub fn copy_to_vec(&self) -> Vec<u8> {
        // this will hard-copy the vec content
        match self.read() {
            Some(slice) => slice.to_vec(),
            None => Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        BufferPool::reset_slice(self.id);
    }

    pub fn try_into_string(&self) -> Result<&str, ErrorKind> {
        if let Some(slice) = self.read() {
            return match str::from_utf8(slice) {
                Ok(raw) => Ok(raw),
                Err(_) => Err(ErrorKind::InvalidData),
            };
        }

        Err(ErrorKind::InvalidData)
    }

    fn len(&self) -> usize {
        BufferPool::slice_stat(self.id, SliceStatusQuery::Length)
    }

    fn capacity(&self) -> usize {
        BufferPool::slice_stat(self.id, SliceStatusQuery::Capacity)
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
