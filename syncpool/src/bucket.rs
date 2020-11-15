#![allow(unused)]

use crate::make_box;
use crate::pool::ElemBuilder;
use crate::utils::{check_len, cpu_relax, enter, make_elem};
use std::mem;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, AtomicPtr, Ordering};

/// Constants
pub(crate) const SLOT_CAP: usize = 8;
const TRIALS_COUNT: usize = 4;

pub(crate) struct Bucket<T> {
    /// the actual data store
    slot: [Option<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: usize,

    /// if the slot is currently being read/write to
    access: AtomicBool,
}

impl<T> Bucket<T> {
    pub(crate) fn new(filler: Option<fn() -> T>) -> Self {
        // create the placeholder
        let mut slice: [Option<T>; SLOT_CAP] = Default::default();

        // fill the placeholder if required
        if let Some(handle) = filler.as_ref() {
            for item in slice.iter_mut() {
                item.replace(handle());
            }
        }

        // done
        Bucket {
            slot: slice,
            len: SLOT_CAP,
            access: AtomicBool::new(false),
        }
    }

    pub(crate) fn size_hint(&self) -> usize {
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
        let val = self.slot[i].take().ok_or(());

        // update internal states if we're good
        if val.is_ok() {
            self.len = i;
        }

        // return the inner value
        val
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a access has
    /// been acquired previously
    pub(crate) fn release(&mut self, mut val: T, reset: Option<fn(&mut T)>) {
        // need to loop over the slots to make sure we're getting the valid value
        let i = self.len;
        if i >= SLOT_CAP {
            return;
        }

        if self.slot[i].is_none() {
            // reset the struct before releasing it to the pool
            if let Some(handle) = reset {
                handle(&mut val);
            }

            // move the value in
            self.slot[i].replace(val);

            // update internal states
            self.len = i + 1;

            // done
            return;
        }

        // if all slots are full, no need to fallback, the `val` will be dropped here
        drop(val);
    }

    /*
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
    */
}

pub(crate) struct Bucket2<T> {
    /// The actual data store. Data are stored in heap and not managed by the runtime, so we must
    /// restore them and drop the data when the bucket is dropped.
    slot: [*mut T; SLOT_CAP],

    /// the current ready-to-use slot count, always offset by 1 to the actual index. This may not be
    /// a real-time reflection of how many elements are actually in the bucket, especially if other
    /// threads are actively interact with the sync pool.
    len: AtomicUsize,

    /// The bitmap of the slots. The implementation rely on the assumption that each bucket only contains
    /// at most 8 elements, otherwise, we need to update the underlying atomic data structure.
    ///
    /// Each position's state are comprised with 2 consecutive bits at (2 * pos) and (2 * pos + 1),
    /// where the bit at (2 * pos) indicates if the slot contains an element (1 -> element; 0 -> empty);
    /// the bit at (2 * pos + 1) indicates if someone is operating at the slot, and hence everyone
    /// else shall avoid using the position, otherwise we may corrupt the underlying data structure.
    bitmap: AtomicU16,
}

impl<T> Bucket2<T> {
    /// Instantiate the bucket and set initial values. If we want to pre-fill the slots, we will also
    /// make sure the bitmap is updated as well.
    pub(crate) fn new(filler: Option<&ElemBuilder<T>>) -> Self {
        // create the placeholder
        let mut slice: [*mut T; SLOT_CAP] = [ptr::null_mut(); SLOT_CAP];
        let mut bitmap: u16 = 0;

        // fill the slots and update the bitmap
        if let Some(handle) = filler {
            for (i, item) in slice.iter_mut().enumerate() {
                *item = Box::into_raw(make_elem(handle));
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

    /// Obtain the number of available elements in this bucket. The size is volatile if the API is
    /// accessed concurrently with read/write, so the
    pub(crate) fn size_hint(&self) -> usize {
        self.len.load(Ordering::Acquire) % (SLOT_CAP + 1)
        //        check_len(self.bitmap.load(Ordering::Acquire))
    }

    /// Try to locate a position where we can fulfil the request -- either grab an element from the
    /// bucket, or put an element back into the bucket. If such a request can't be done, we will
    /// return error.
    pub(crate) fn access(&self, get: bool) -> Result<usize, ()> {
        // register intentions first, make sure the len is in post-action state so it can reject
        // future or concurrent attempts if it's unlikely to succeed in this slot.
        let curr_len = if get {
            self.len.fetch_sub(1, Ordering::Relaxed)
        } else {
            self.len.fetch_add(1, Ordering::Relaxed)
        };

        // oops, last op blew off the roof, back off mate. Note that (0 - 1 == MAX_USIZE) for stack
        // overflow, still way off the roof and a proof of not doing well.
        if curr_len > SLOT_CAP || (get && curr_len == 0) {
            return self.access_failure(get);
        }

        // try 2 times on this slot if the desired slot happens to be taken ...
        let mut trials: usize = TRIALS_COUNT;
        while trials > 0 {
            trials -= 1;

            // init try
            let (pos, mask) = match enter(self.bitmap.load(Ordering::Acquire), get) {
                Ok(pos) => (pos, 0b10 << (2 * pos)),
                Err(()) => continue,
            };

            // main loop to try to update the bitmap
            let old = self.bitmap.fetch_or(mask, Ordering::AcqRel);

            // if the lock bit we replaced was not yet marked at the atomic op, we're good
            if old & mask == 0 {
                return Ok(pos as usize);
            }

            // otherwise, try again after some wait. The earliest registered gets some favor by
            // checking and trying to lodge a position more frequently than the later ones.
            cpu_relax(trials + 1);
        }

        self.access_failure(get)
    }

    /// Update the bitmap to make sure: 1) the lock bit of the operated upon position is flipped back
    /// to free-to-use; 2) the marker bit of the operated upon position is properly updated. We should
    /// succeed at the first trial of the for-loop, otherwise we may in trouble.
    pub(crate) fn leave(&self, pos: u16) {
        // the lock bit we want to toggle
        let lock_bit = 0b10 << (2 * pos);

        loop {
            // update both lock bit and the slot bit
            let old = self.bitmap.fetch_xor(0b11 << (2 * pos), Ordering::SeqCst);
            if old & lock_bit == lock_bit {
                return;
            }
        }
    }

    /// Locate the element from the desired position. The API will return an error if such operation
    /// can't be accomplished, such as the destination doesn't contain a element, or the desired position
    /// is OOB.
    ///
    /// The function is safe because it's used internally, and each time it's guaranteed an exclusive
    /// access has been acquired previously.
    pub(crate) fn checkout(&mut self, pos: usize) -> Result<Box<T>, ()> {
        // check the boundary and underlying slot position before doing something with it.
        if pos >= SLOT_CAP || self.slot[pos].is_null() {
            return Err(());
        }

        // swap the pointer out of the slot, this is the raw pointer to the heap memory location of
        // the underlying data. The swap operation is cheap, since *mut T is guaranteed to be 8-bytes
        // in length and hence we'll run the "simplified" version of the mem swap which is cheaper
        // to run.
        let val = mem::replace(&mut self.slot[pos], ptr::null_mut());

        // Restore to the box version, this won't allocate since the pointed to content already
        // exist. This action is safe since all values we put behind the pointers are knocked out
        // from its boxed version, guaranteed by the implementation of the `new` and `release` APIs.
        Ok(unsafe { Box::from_raw(val) })
    }

    /// Release the element back into the pool. If a reset function has been previously provided, we
    /// will call the function to reset the value before putting it back. The API will be no-op if
    /// the desired operation can't be conducted, such as if the position is OOB, or the position
    /// already contains an element.
    ///
    /// The function is safe because it's used internally, and each time it's guaranteed an exclusive
    /// access has been acquired previously
    pub(crate) fn release(&mut self, pos: usize, mut val: Box<T>, reset: Option<fn(&mut T)>) {
        // check if the slot has already been occupied (unlikely but still)
        if pos >= SLOT_CAP || !self.slot[pos].is_null() {
            return;
        }

        // reset the struct before releasing it to the pool
        if let Some(handle) = reset {
            handle(&mut val);
        }

        // move the value in
        self.slot[pos] = Box::into_raw(val);
    }

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

impl<T> Drop for Bucket2<T> {
    fn drop(&mut self) {
        for item in self.slot.iter_mut() {
            if item.is_null() {
                continue;
            }

            unsafe {
                ptr::drop_in_place(*item);
            }
            *item = ptr::null_mut();
        }
    }
}

unsafe impl<T> Send for Bucket2<T> {}

pub(crate) struct RingBucket<T> {
    /// The actual data store. Data are stored in heap and not managed by the runtime, so we must
    /// restore them and drop the data when the bucket is dropped.
    slot: [AtomicPtr<T>; SLOT_CAP],

    /// the current ready-to-use slot count, always offset by 1 to the actual index. This may not be
    /// a real-time reflection of how many elements are actually in the bucket, especially if other
    /// threads are actively interact with the sync pool.
    len: AtomicUsize,

    head: AtomicUsize,

    tail: AtomicUsize,
}

impl<T> RingBucket<T> {
    /// Instantiate the bucket and set initial values. If we want to pre-fill the slots, we will also
    /// make sure the bitmap is updated as well.
    pub(crate) fn new(filler: Option<&ElemBuilder<T>>) -> Self {
        // create the placeholder
        let mut slice: [AtomicPtr<T>; SLOT_CAP] = Default::default();

        // fill the slots and update the bitmap
        if let Some(handle) = filler {
            for (_, item) in slice.iter_mut().enumerate() {
                item.swap(Box::into_raw(make_elem(handle)), Ordering::SeqCst);
            }
        }

        // done
        RingBucket {
            slot: slice,
            len: AtomicUsize::new(SLOT_CAP),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(SLOT_CAP),
        }
    }
}
