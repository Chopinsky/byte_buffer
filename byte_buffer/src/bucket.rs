#![allow(unused)]

use crate::utils::make_buffer;
use std::sync::atomic::{AtomicU16, Ordering};
use std::u16;
use std::ptr::NonNull;

const CAPACITY: usize = 16;

pub(crate) struct Bucket {
    stores: Box<[*mut u8]>,
    bitmap: AtomicU16,
    next: Option<*mut Bucket>,
}

impl Bucket {
    pub(crate) fn new(size: usize) -> Self {
        let mut base = Vec::with_capacity(CAPACITY);

        (0..16).for_each(|_| {
            let buf = make_buffer(size);
            base.push(buf);
        });

        Bucket {
            stores: base.into_boxed_slice(),
            bitmap: AtomicU16::new(u16::MAX),
            next: None, //ptr::null_mut(),
        }
    }

    pub(crate) fn checkout(&mut self) -> Option<Vec<u8>> {
        let mut tries: u8 = 4;
        let mut base: u16 = self.bitmap.load(Ordering::Acquire);

        while base != 0 && tries > 0 {
            let pos = base.trailing_zeros() as u16;

            if let Err(old) = self.bitmap.compare_exchange(
                base,
                base ^ (1u16 << pos),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                // the bitmap has just been updated, loop back to try again
                base = old;
                tries -= 1;
                continue;
            }

            // try to take this buf
            return Some(self.get_buf(pos as usize));
        }

        None
    }

    pub(crate) fn next(&mut self) -> Option<&mut Bucket> {
        self.next.map(|ptr| {
            unsafe { &mut *ptr }
        })
    }

    fn get_buf(&self, pos: usize) -> Vec<u8> {
        assert!(pos < 16);

        unsafe { Vec::from_raw_parts(self.stores[pos], 0, CAPACITY) }
    }
}