#![allow(unused)]

use crate::utils::{cpu_relax, enter};
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};

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

        // fill the placeholder if required
        if fill {
            for item in slice.iter_mut() {
                item.replace(Default::default());
            }
        }

        // done
        Bucket {
            slot: slice,
            len: SLOT_CAP,
            access: AtomicBool::new(false),
        }
    }

    pub(crate) fn len_hint(&self) -> usize {
        self.len % (SLOT_CAP + 1)
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
    slot: [UnsafeCell<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: AtomicUsize,

    bitmap: AtomicU16,
}

impl<T: Default> Bucket2<T> {
    pub(crate) fn new(fill: bool) -> Self {
        // create the placeholder
        let mut slice: [UnsafeCell<T>; SLOT_CAP] = unsafe { MaybeUninit::zeroed().assume_init() };
        let mut bitmap: u16 = 0;

        // fill the slots and update the bitmap
        if fill {
            for (i, item) in slice.iter_mut().enumerate() {
                unsafe {
                    item.get().write(Default::default());
                }
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

    pub(crate) fn len_hint(&self) -> usize {
        //        println!("{:#018b}", self.bitmap.load(Ordering::Acquire));
        self.len.load(Ordering::Acquire) % (SLOT_CAP + 1)
    }

    pub(crate) fn access(&self, get: bool) -> Result<usize, ()> {
        // pre-checkout, make sure the len is in post-action state so it can reject future attempts
        // if it's unlikely to succeed in this slot.
        let curr_len = if get {
            self.len.fetch_sub(1, Ordering::AcqRel)
        } else {
            self.len.fetch_add(1, Ordering::AcqRel)
        };

        // oops, last op blew off the roof, back off mate. Note that (0 - 1 == MAX_USIZE) for stack
        // overflow, still way off the roof and a proof of not doing well.
        if curr_len > SLOT_CAP || (get && curr_len == 0) {
            return self.access_failure(get);
        }

        let mut trials: usize = 2;
        while trials > 0 {
            // init try
            let (pos, mask) = match enter(self.bitmap.load(Ordering::Acquire), get) {
                Ok(pos) => (pos, 0b10 << (2 * pos)),
                Err(()) => {
                    return self.access_failure(get);
                }
            };

            // main loop to try to update the bitmap
            let old = self.bitmap.fetch_or(mask, Ordering::AcqRel);

            // if the lock bit we replaced was not yet marked at the atomic op, we're good
            if old & mask == 0 {
                return Ok(pos as usize);
            }

            // otherwise, try again after some wait
            cpu_relax(4 * trials);
            trials -= 1;
        }

        self.access_failure(get)
    }

    pub(crate) fn leave(&self, pos: u16) {
        let lock_bit = 0b10 << (2 * pos);

        loop {
            let old = self.bitmap.fetch_xor(0b11 << (2 * pos), Ordering::SeqCst);
            if old & lock_bit == lock_bit {
                return;
            }
        }
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn checkout(&mut self, pos: usize) -> Result<T, ()> {
        // return the value
        Ok(swap_out(&self.slot[pos]))
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn release(&mut self, pos: usize, mut val: T, reset: *mut ResetHandle<T>) {
        // need to loop over the slots to make sure we're getting the valid value
        if pos >= SLOT_CAP {
            return;
        }

        // reset the struct before releasing it to the pool
        if !reset.is_null() {
            unsafe {
                (*reset)(&mut val);
            }
        }

        // move the value in
        swap_in(&self.slot[pos], val);
    }

    /*    pub(crate) fn debug(&self) {
        println!("{:#018b}", self.bitmap.load(Ordering::SeqCst));
    }*/

    #[inline]
    fn access_failure(&self, get: bool) -> Result<usize, ()> {
        if get {
            self.len.fetch_add(1, Ordering::AcqRel);
        } else {
            self.len.fetch_sub(1, Ordering::AcqRel);
        }

        Err(())
    }
}

fn swap_in<T: Default>(container: &UnsafeCell<T>, content: T) {
    unsafe {
        container.get().write(content);
    }
}

fn swap_out<T: Default>(container: &UnsafeCell<T>) -> T {
    unsafe { container.get().read() }
}
