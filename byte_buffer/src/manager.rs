#![allow(dead_code)]

use crate::buffer::{BufferPool, PoolManagement};
use crate::channel::{self as channel};
use crate::utils::*;
use std::io::ErrorKind;
use std::str;
use std::sync::Once;
use std::thread;
use std::vec;

static ONCE: Once = Once::new();

pub struct ByteBuffer;

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

            BufferPool::make(store, capacity, sender);
        });
    }

    pub fn slice() -> BufferSlice {
        match BufferPool::exec(BufOp::Reserve(true)) {
            Some(val) => BufferSlice {
                id: val,
                fallback: None,
                dirty: false,
            },
            None => BufferSlice {
                id: 0,
                fallback: Some(vec::from_elem(0, BufferPool::default_capacity())),
                dirty: false,
            },
        }
    }

    pub fn try_slice() -> Option<BufferSlice> {
        BufferPool::exec(BufOp::Reserve(false)).and_then(|id| {
            Some(BufferSlice {
                id,
                fallback: None,
                dirty: false,
            })
        })
    }

    #[inline]
    pub fn extend(additional: usize) {
        BufferPool::exec(BufOp::Extend(additional));
    }
}

pub struct BufferSlice {
    id: usize,
    fallback: Option<Vec<u8>>,
    dirty: bool,
}

impl BufferSlice {
    pub(crate) fn new(id: usize, fallback: Option<Vec<u8>>) -> Self {
        BufferSlice {
            id,
            fallback,
            dirty: false,
        }
    }

    pub fn as_writable(&mut self) -> &mut [u8] {
        self.dirty = true;

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
            }
        }
    }

    pub fn as_writable_vec(&mut self) -> &mut Vec<u8> {
        self.dirty = true;

        if let Some(ref mut vec) = self.fallback {
            return vec;
        }

        match BufferPool::get_writable(self.id) {
            Ok(vec) => vec,
            Err(_) => {
                self.fallback = Some(vec::from_elem(0, BufferPool::default_capacity()));
                if let Some(vec) = self.fallback.as_mut() {
                    return vec;
                }

                unreachable!();
            }
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
            }
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
            }
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
        if !self.dirty {
            return;
        }

        BufferPool::reset_slice(self.id);

        if let Some(fb) = self.fallback.as_mut() {
            fb.iter_mut().for_each(|val| *val = 0);
        }
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
            BufferPool::exec(BufOp::ReleaseAndExtend(
                self.fallback.take().unwrap(),
                self.dirty,
            ));
        } else {
            BufferPool::reset_and_release(self.id, self.dirty);
        }
    }
}
