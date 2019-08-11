#![allow(unused)]

use crate::utils::{cpu_relax, enter, exit};
use std::fmt::Error;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU16, AtomicUsize, Ordering};

/// Constants
pub(crate) const SLOT_CAP: usize = 8;
pub(crate) type ResetHandle<T> = fn(&mut T);

pub(crate) struct Bucket<T> {
    /// the actual data store
    slot: [Option<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: usize,

    /// if the slot is currently being read/write to
    access: AtomicBool,
}

impl<T: Default> Bucket<T> {
    pub(crate) fn new(fill: bool) -> Self {
        // create the placeholder
        let mut slice: [Option<T>; SLOT_CAP] = unsafe { MaybeUninit::zeroed().assume_init() };
        let mut bitmap: u16 = 0;

        // fill the placeholder if required
        if fill {
            for item in slice.iter_mut() {
                item.replace(Default::default());
            }

            // fill the slots
            for i in 0..(SLOT_CAP - 1) {
                bitmap |= 1 << (2 * i as u16 + 1);
            }

            // update the last slot
            bitmap |= 1;
        }

        // done
        Bucket {
            slot: slice,
            len: SLOT_CAP,
            access: AtomicBool::new(false),
        }
    }

    pub(crate) fn access(&self, get: bool) -> bool {
        // count down to lock timeout
        let mut count = if get { 4 } else { 2 };

        // check the access and wait if not available
        while self
            .access
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Acquire)
            .is_err()
        {
            if count == 0 {
                return false;
            }

            cpu_relax(2 * count);
            count -= 1;
        }

        if (get && self.len == 0) || (!get && self.len == SLOT_CAP) {
            // not actually locked
            self.leave();

            // read but empty, or write but full, all fail
            return false;
        }

        true
    }

    pub(crate) fn leave(&self) {
        self.access.store(false, Ordering::Release);
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn checkout(&mut self) -> Result<T, ()> {
        // need to loop over the slots to make sure we're getting the valid value, starting from
        let i = self.len - 1;
        if self.slot[i].is_none() {
            return Err(());
        }

        // update internal states
        self.len = i;

        // return the value
        Ok(self.swap_out(i))
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn release(&mut self, mut val: T, reset: *mut ResetHandle<T>) {
        // need to loop over the slots to make sure we're getting the valid value
        let i = self.len;
        if i >= SLOT_CAP {
            return;
        }

        if self.slot[i].is_none() {
            // reset the struct before releasing it to the pool
            if !reset.is_null() {
                unsafe {
                    (*reset)(&mut val);
                }
            }

            // move the value in
            self.swap_in(i, val);

            // update internal states
            self.len = i + 1;

            // done
            return;
        }

        // if all slots are full, no need to fallback, the `val` will be dropped here
        drop(val);
    }

    fn swap_in(&mut self, index: usize, content: T) {
        let src = &mut self.slot[index] as *mut Option<T>;
        unsafe {
            src.write(Some(content));
        }
    }

    fn swap_out(&mut self, index: usize) -> T {
        let src = &mut self.slot[index] as *mut Option<T>;

        unsafe {
            // save off the old values
            let val = ptr::read(src).unwrap_or_default();

            // swap values
            src.write(None);

            val
        }
    }
}

pub(crate) struct Bucket2<T> {
    /// the actual data store
    slot: [Option<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: AtomicUsize,

    bitmap: AtomicU16,
}

impl<T: Default> Bucket2<T> {
    pub(crate) fn new(fill: bool) -> Self {
        // create the placeholder
        let mut slice: [Option<T>; SLOT_CAP] = unsafe { MaybeUninit::zeroed().assume_init() };
        let mut bitmap: u16 = 0;

        // fill the slots and update the bitmap
        if fill {
            for (i, item) in slice.iter_mut().enumerate() {
                item.replace(Default::default());
                bitmap |= 1 << (2 * i as u16);
            }
        }

        // done
        Bucket2 {
            slot: slice,
            len: AtomicUsize::new(SLOT_CAP),
            bitmap: AtomicU16::new(bitmap),
        }
    }

    pub(crate) fn access(&self, get: bool) -> Result<usize, ()> {
        // gate keeper: quick rejection if the condition won't match
        let curr_len = self.len.load(Ordering::SeqCst);
        if (get && curr_len == 0) || (!get && curr_len >= SLOT_CAP) {
            return Err(());
        }

        //TODO: instead of updating self.len, use visitor counter to accomplish the task

        // pre-checkout, make sure the len is in post-action state so it can reject future attempts
        // if it's unlikely to succeed in this slot.
        let (curr_len, mut trials) = if get {
            (self.len.fetch_sub(1, Ordering::AcqRel), 4)
        } else {
            (self.len.fetch_add(1, Ordering::AcqRel), 2)
        };

        // oops, last op blew off the roof, back off mate. Note that (0 - 1 == MAX_USIZE) for stack
        // overflow, still way off the roof and a proof of not doing well.
        if curr_len > SLOT_CAP {
//            println!("No luck with {} ... first fail: {}", if get { "get" } else { "put" }, curr_len);
            return self.access_failure(get);
        }

        while trials > 0 {
            // init try
            let pos = match enter(self.bitmap.load(Ordering::Acquire), get) {
                Ok(result) => result,
                Err(()) => {

//                    println!("No luck with {} ... second fail", if get { "get" } else { "put" });

                    return self.access_failure(get)
                },
            };

            // main loop to try to update the bitmap
            let mask = 0b10 << (2 * pos);
            let old = self.bitmap.fetch_or(mask, Ordering::AcqRel);

            // if the lock bit we replaced was not yet marked at the atomic op, we're good
            if old & mask == 0 {
                return Ok(pos as usize)
            }

            // otherwise, try again.
            trials -= 1;
        }

//        println!("No luck with {} ... final fail", if get { "get" } else { "put" });
        self.access_failure(get)
    }

    pub(crate) fn leave(&self, pos: usize) {
        let mask = 0b10 << (2 * pos as u16);

//        let padded = 2 * pos as u16;
//        if self.bitmap.load(Ordering::Acquire) & (0b10 << padded) == 0 {
//            // bit already marked as free-to-use
//            return;
//        }

        loop {
            let old = self.bitmap.fetch_xor(mask, Ordering::SeqCst);
            if old & mask == mask {
                return;
            }
        }
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn checkout(&mut self, pos: usize) -> Result<T, ()> {
        // check if it's a valid position to swap out the value
        if self.slot[pos].is_none() {
            return Err(());
        }

        // return the value
        Ok(self.swap_out(pos))
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn release(&mut self, pos: usize, mut val: T, reset: *mut ResetHandle<T>) {
        // need to loop over the slots to make sure we're getting the valid value
        if pos >= SLOT_CAP {
            return;
        }

        if self.slot[pos].is_none() {
            // reset the struct before releasing it to the pool
            if !reset.is_null() {
                unsafe {
                    (*reset)(&mut val);
                }
            }

            // move the value in
            self.swap_in(pos, val);

            // done
            return;
        }

        // if all slots are full, no need to fallback, the `val` will be dropped here
        drop(val);
    }

    pub(crate) fn debug(&self) {
        println!("{:#018b}", self.bitmap.load(Ordering::SeqCst));
    }

    #[inline]
    fn access_failure(&self, get: bool) -> Result<usize, ()> {
//        println!("No luck with {} ...", if get { "get" } else { "put" });

        if get {
            self.len.fetch_add(1, Ordering::AcqRel);
        } else {
            self.len.fetch_sub(1, Ordering::AcqRel);
        }

        Err(())
    }

    fn swap_in(&mut self, index: usize, content: T) {
        let src = &mut self.slot[index] as *mut Option<T>;
        unsafe {
            src.write(Some(content));
        }
    }

    fn swap_out(&mut self, index: usize) -> T {
        let src = &mut self.slot[index] as *mut Option<T>;

        unsafe {
            // save off the old values
            let val = ptr::read(src).unwrap_or_default();

            // swap values
            src.write(None);

            val
        }
    }
}
