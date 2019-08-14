#![allow(unused)]

use crate::bucket::*;
use crate::utils::cpu_relax;
use std::ops::Deref;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

const POOL_SIZE: usize = 8;
const EXPANSION_CAP: usize = 512;
const SPIN_PERIOD: usize = 4;

/// Configuration flags
const CONFIG_ALLOW_EXPANSION: usize = 1;

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
    slots: Vec<Bucket2<T>>,

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
    miss_count: AtomicUsize,

    /// if we allow expansion of the pool
    configure: AtomicUsize,

    /// the handle to be invoked before putting the struct back
    reset_handle: AtomicPtr<ResetHandle<T>>,
}

impl<T: Default> SyncPool<T> {
    pub fn new() -> Self {
        Self::make_pool(POOL_SIZE)
    }

    pub fn with_size(size: usize) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size)
    }

    pub fn len(&self) -> usize {
        self.slots.iter().fold(0, |sum, item| sum + item.len_hint())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&mut self) -> Box<T> {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.fetch_add(1, Ordering::AcqRel) % cap;

        let mut pos = origin;
        let mut trials = cap;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the access or move on
            if let Ok(i) = slot.access(true) {
                // try to checkout one slot
                let checkout = slot.checkout(i);
                slot.leave(i as u16);

/*            if slot.access(true) {
                // try to checkout one slot
                let checkout = slot.checkout();
                slot.leave();*/

                if let Ok(val) = checkout {
                    // now we're locked, get the val and update internal states
                    self.curr.fetch_add(pos, Ordering::Release);

                    // done
                    return val;
                }

                // failed to checkout, break and let the remainder logic to handle the rest
                break;
            }

            // hold off a bit to reduce contentions
            cpu_relax(SPIN_PERIOD);

            // update to the next position now.
            pos = self.curr.fetch_add(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                break;
            }
        }

        // make sure our guard has been returned if we want the correct visitor count
        drop(_guard);

        self.miss_count.fetch_add(1, Ordering::Relaxed);
        Default::default()
    }

    pub fn put(&mut self, val: Box<T>) {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.load(Ordering::Acquire) % cap;

        let mut pos = origin;
        let mut trials = cap;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the access or move on
            if let Ok(i) = slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.store(pos, Ordering::Release);

                // put the value back and reset
                slot.release(i, val, self.reset_handle.load(Ordering::Acquire));
                slot.leave(i as u16);

                return;
            }

/*            if slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.store(pos, Ordering::Release);

                // put the value back into the slot
                slot.release(val, self.reset_handle.load(Ordering::Acquire));
                slot.leave();

                return;
            }*/

            // hold off a bit to reduce contentions
            cpu_relax(SPIN_PERIOD / 2);

            // update states
            pos = self.curr.fetch_sub(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                break;
            }
        }
    }

    fn make_pool(size: usize) -> Self {
        let mut pool = SyncPool {
            slots: Vec::with_capacity(size),
            curr: AtomicUsize::new(0),
            visitor_counter: (AtomicUsize::new(1), AtomicBool::new(false)),
            miss_count: AtomicUsize::new(0),
            configure: AtomicUsize::new(0),
            reset_handle: AtomicPtr::new(ptr::null_mut()),
        };

        pool.add_slots(size, true);
        pool
    }

    #[inline]
    fn add_slots(&mut self, count: usize, fill: bool) {
        (0..count).for_each(|_| {
//            self.slots.push(Bucket::new(fill));
            self.slots.push(Bucket2::new(fill));
        });
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
        SyncPool::new()
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
    fn miss_count(&self) -> usize;
}

impl<T> PoolState for SyncPool<T> {
    fn expansion_enabled(&self) -> bool {
        let configure = self.configure.load(Ordering::SeqCst);
        configure & CONFIG_ALLOW_EXPANSION > 0
    }

    fn miss_count(&self) -> usize {
        self.miss_count.load(Ordering::Acquire)
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
            self.add_slots(additional, true);
            self.miss_count.store(0, Ordering::Release);
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
