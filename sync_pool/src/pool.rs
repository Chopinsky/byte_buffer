#![allow(unused)]

use crate::utils::{cpu_relax, enter, exit};
use std::fmt::Error;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU16, AtomicUsize, Ordering};

/// Constants
pub(crate) const SLOT_CAP: usize = 8;
const POOL_SIZE: usize = 8;
const EXPANSION_CAP: usize = 512;

/// Configuration flags
const CONFIG_ALLOW_EXPANSION: usize = 1;

type ResetHandle<T> = fn(&mut T);

struct Slot<T> {
    /// the actual data store
    slot: [Option<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: usize,

    /// if the slot is currently being read/write to
    access: AtomicBool,
}

impl<T: Default> Slot<T> {
    fn new(fill: bool) -> Self {
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
        Slot {
            slot: slice,
            len: SLOT_CAP,
            access: AtomicBool::new(false),
        }
    }

    fn try_lock(&self, get: bool) -> bool {
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
            self.unlock();

            // read but empty, or write but full, all fail
            return false;
        }

        true
    }

    fn unlock(&self) {
        self.access.store(false, Ordering::Release);
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a try_lock has
    /// been acquired previously
    fn checkout(&mut self) -> Result<T, ()> {
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

    /// The function is safe because it's used internally, and each time it's guaranteed a try_lock has
    /// been acquired previously
    fn release(&mut self, mut val: T, reset: *mut ResetHandle<T>) {
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

struct Slot2<T> {
    /// the actual data store
    slot: [Option<T>; SLOT_CAP],

    /// the current ready-to-use slot index, always offset by 1 to the actual index
    len: AtomicUsize,

    bitmap: AtomicU16,
}

impl<T: Default> Slot2<T> {
    fn new(fill: bool) -> Self {
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
        Slot2 {
            slot: slice,
            len: AtomicUsize::new(SLOT_CAP),
            bitmap: AtomicU16::new(bitmap),
        }
    }

    fn access(&self, get: bool) -> Result<usize, ()> {
        // quick rejection if the condition won't match
        let curr_len = self.len.load(Ordering::SeqCst);
        if (get && curr_len == 0) || (!get && curr_len == SLOT_CAP) {
            return Err(());
        }

        // pre-checkout, make sure the len is in post-action state so it can reject future attempts
        // if it's unlikely to succeed in this slot.
        let mut trials = if get {
            self.len.fetch_add(1, Ordering::AcqRel);
            4
        } else {
            self.len.fetch_sub(1, Ordering::AcqRel);
            2
        };

        let mut start = self.bitmap.load(Ordering::Acquire);
        let (mut target, mut pos) = match enter(start, get) {
            Ok(result) => (result.0, result.1),
            Err(()) => return self.access_failure(get),
        };

        // main loop to try to update the bitmap
        while let Err(next) =
            self.bitmap
                .compare_exchange(start, target, Ordering::Acquire, Ordering::Acquire)
        {
            // timeout, try next slot
            if trials == 0 {
                return self.access_failure(get);
            }

            trials -= 1;
            start = next;

            match enter(start, get) {
                Ok(result) => {
                    target = result.0;
                    pos = result.1;
                }
                Err(()) => return self.access_failure(get),
            };
        }

        Ok(pos as usize)
    }

    fn leave(&self, pos: usize) {
        let pad_pos = 2 * pos as u16;
        if self.bitmap.load(Ordering::Acquire) & (0b10 << pad_pos) == 0 {
            // bit already marked as free-to-use
            return;
        }

        self.bitmap.fetch_xor(0b11 << pad_pos, Ordering::SeqCst);
    }

    #[inline]
    fn access_failure(&self, get: bool) -> Result<usize, ()> {
//        println!("No luck with {} ...", if get { "get" } else { "put" });

        if get {
            self.len.fetch_sub(1, Ordering::AcqRel);
        } else {
            self.len.fetch_add(1, Ordering::AcqRel);
        }

        Err(())
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a try_lock has
    /// been acquired previously
    fn checkout(&mut self, pos: usize) -> Result<T, ()> {
        // check if it's a valid position to swap out the value
        if self.slot[pos].is_none() {
            return Err(());
        }

        // return the value
        Ok(self.swap_out(pos))
    }

    /// The function is safe because it's used internally, and each time it's guaranteed a try_lock has
    /// been acquired previously
    fn release(&mut self, pos: usize, mut val: T, reset: *mut ResetHandle<T>) {
        // need to loop over the slots to make sure we're getting the valid value
        if pos >= SLOT_CAP {
            return;
        }

        if self.slot[pos].is_none() {
            // reset the struct before releasing it to the pool
            if !reset.is_null() {
                unsafe { (*reset)(&mut val); }
            }

            // move the value in
            self.swap_in(pos, val);

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

struct VisitorGuard<'a>(&'a AtomicUsize);

impl<'a> VisitorGuard<'a> {
    fn register(base: &'a (AtomicUsize, AtomicBool)) -> Self {
        let mut count = 0;

        // wait if the underlying storage is in protection mode
        while base.1.load(Ordering::Acquire) {
            cpu_relax(count + 8);

            if count < 8 {
                count += 1;
            }
        }

        base.0.fetch_add(1, Ordering::SeqCst);
        VisitorGuard(&base.0)
    }
}

impl<'a> Drop for VisitorGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct SyncPool<T> {
    /// The slots storage
    slots: Vec<Slot2<T>>,

    /// the next channel to try
    curr: AtomicUsize,

    /// First node: how many threads are concurrently accessing the struct:
    ///   0   -> updating the `slots` field;
    ///   1   -> no one is using the pool;
    ///   num -> number of visitors
    /// Second node: write barrier:
    ///   true  -> write barrier raised
    ///   false -> no write barrier
    visitor_counter: (AtomicUsize, AtomicBool),

    /// the number of times we failed to find an in-store struct to offer
    fault_count: AtomicUsize,

    /// if we allow expansion of the pool
    configure: AtomicUsize,

    /// the handle to be invoked before putting the struct back
    reset_handle: AtomicPtr<ResetHandle<T>>,
}

impl<T: Default> SyncPool<T> {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_size(size: usize) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size)
    }

    pub fn get(&mut self) -> T {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.fetch_add(1, Ordering::AcqRel) % cap;

        let mut pos = origin;
        let mut trials = cap / 2;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the try_lock or move on
            if let Ok(i) = slot.access(true) {
                // try to checkout one slot
                let checkout = slot.checkout(i);
                slot.leave(i);

                if let Ok(val) = checkout {
                    // now we're locked, get the val and update internal states
                    self.curr.store(pos, Ordering::Release);

                    // done
                    return val;
                }

                // failed to checkout, break and let the remainder logic to handle the rest
                break;
            }

/*            if slot.try_lock(true) {
                // try to checkout one slot
                let checkout = slot.checkout();
                slot.unlock();

                if let Ok(val) = checkout {
                    // now we're locked, get the val and update internal states
                    self.curr.store(pos, Ordering::Release);

                    // done
                    return val;
                }

                // failed to checkout, break and let the remainder logic to handle the rest
                break;
            }*/

            // update to the next position now.
            pos = self.curr.fetch_add(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 || pos == origin {
                break;
            }
        }

        // make sure our guard has been returned if we want the correct visitor count
        drop(_guard);

        Default::default()
    }

    pub fn put(&mut self, val: T) {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.load(Ordering::Acquire) % cap;

        let mut pos = origin;
        let mut trials = cap / 2;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the try_lock or move on
            if let Ok(i) = slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.store(pos, Ordering::Release);

                // put the value back and reset
                slot.release(i, val, self.reset_handle.load(Ordering::Acquire));
                slot.leave(i);

                return;
            }

/*            if slot.try_lock(false) {
                // now we're locked, get the val and update internal states
                self.curr.store(pos, Ordering::Release);

                // put the value back into the slot
                slot.release(val, self.reset_handle.load(Ordering::Acquire));
                slot.unlock();

                return;
            }*/

            // update states
            pos = self.curr.fetch_sub(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 || pos == origin {
                break;
            }
        }
    }

    fn make_pool(size: usize) -> Self {
        let mut s = Vec::with_capacity(size);

        (0..size).for_each(|_| {
            // add the slice back to the vec container
            s.push(Slot2::new(true));
        });

        SyncPool {
            slots: s,
            curr: AtomicUsize::new(0),
            visitor_counter: (AtomicUsize::new(1), AtomicBool::new(false)),
            fault_count: AtomicUsize::new(0),
            configure: AtomicUsize::new(0),
            reset_handle: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn update_config(&mut self, mask: usize, target: bool) {
        let mut curr = self.configure.load(Ordering::SeqCst);

        while let Err(old) =
            self.configure
                .compare_exchange(curr, curr ^ mask, Ordering::SeqCst, Ordering::Relaxed)
        {
            if !((old & mask > 0) ^ target) {
                // the configure already matches, we're done
                return;
            }

            curr = old;
        }
    }
}

impl<T> Default for SyncPool<T>
where
    T: Default,
{
    fn default() -> Self {
        SyncPool::make_pool(POOL_SIZE)
    }
}

impl<T> Drop for SyncPool<T> {
    fn drop(&mut self) {
        self.slots.clear();

        unsafe {
            // now drop the reset handle if it's not null
            Box::from_raw(self.reset_handle.swap(ptr::null_mut(), Ordering::SeqCst));
        }
    }
}

pub trait PoolState {
    fn expansion_enabled(&self) -> bool;
    fn fault_count(&self) -> usize;
}

impl<T> PoolState for SyncPool<T> {
    fn expansion_enabled(&self) -> bool {
        let configure = self.configure.load(Ordering::SeqCst);
        configure & CONFIG_ALLOW_EXPANSION > 0
    }

    fn fault_count(&self) -> usize {
        self.fault_count.load(Ordering::Acquire)
    }
}

pub trait PoolManager<T> {
    fn allow_expansion(&mut self, allow: bool);
    fn expand(&mut self, additional: usize, block: bool) -> bool;
    fn reset_handle(&mut self, handle: ResetHandle<T>);
}

impl<T> PoolManager<T> for SyncPool<T>
where
    T: Default,
{
    fn allow_expansion(&mut self, allow: bool) {
        if !(self.expansion_enabled() ^ allow) {
            // not flipping the configuration, return
            return;
        }

        self.update_config(CONFIG_ALLOW_EXPANSION, allow);
    }

    fn expand(&mut self, additional: usize, block: bool) -> bool {
        // if the pool isn't allowed to expand, just return
        if !self.expansion_enabled() {
            return false;
        }

        // if exceeding the upper limit, quit
        if self.slots.len() > EXPANSION_CAP {
            return false;
        }

        // raise the write barrier now, if someone has already raised the flag to indicate the
        // intention to write, let me go away.
        if self
            .visitor_counter
            .1
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        // busy waiting ... for all visitors to leave
        let mut count: usize = 0;
        let safe = loop {
            match self
                .visitor_counter
                .0
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::Relaxed)
            {
                Ok(_) => break true,
                Err(_) => {
                    cpu_relax(2);
                    count += 1;

                    if count > 8 && !block {
                        break false;
                    }
                }
            }
        };

        if safe {
            // update the slots by pushing `additional` slots
            (0..additional).for_each(|_| {
                self.slots.push(Slot2::new(true));
            });

            self.fault_count.store(0, Ordering::Release);
        }

        // update the internal states
        self.visitor_counter.0.store(1, Ordering::SeqCst);
        self.visitor_counter.1.store(false, Ordering::Release);

        safe
    }

    fn reset_handle(&mut self, handle: ResetHandle<T>) {
        let h = Box::new(handle);
        self.reset_handle
            .swap(Box::into_raw(h) as *mut ResetHandle<T>, Ordering::Release);
    }
}
