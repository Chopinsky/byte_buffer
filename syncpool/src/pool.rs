use crate::bucket::*;
use crate::utils::cpu_relax;
use std::ops::Add;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Instant, Duration};

const POOL_SIZE: usize = 8;
const EXPANSION_CAP: usize = 512;
const SPIN_PERIOD: usize = 4;

/// Configuration flags
const CONFIG_ALLOW_EXPANSION: usize = 1;

struct VisitorGuard<'a>(&'a AtomicUsize);

impl<'a> VisitorGuard<'a> {
    fn register(base: &'a (AtomicUsize, AtomicBool)) -> Self {
        let mut count = 8;

        // wait if the underlying storage is in protection mode
        while base.1.load(Ordering::Relaxed) {
            cpu_relax(count);

            if count > 2 {
                count -= 1;
            }
        }

        base.0.fetch_add(1, Ordering::AcqRel);
        VisitorGuard(&base.0)
    }
}

impl<'a> Drop for VisitorGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct SyncPool<T> {
    /// The slots storage
    slots: Vec<Bucket2<T>>,

    /// the next bucket to try
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
    reset_handle: Option<fn(&mut T)>,
}

impl<T: Default> SyncPool<T> {
    /// Create a pool with default size of 64 pre-allocated elements in it.
    pub fn new() -> Self {
        Self::make_pool(POOL_SIZE)
    }

    /// Create a `SyncPool` with pre-defined number of elements. Note that we will round-up
    /// the size such that the total number of elements in the pool will mod to 8.
    pub fn with_size(size: usize) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size)
    }

    /// Try to obtain a pre-allocated element from the pool. This method will always succeed even if
    /// the pool is empty or not available for anyone to access, and in this case, a new boxed-element
    /// will be created.
    pub fn get(&mut self) -> Box<T> {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let origin: usize = self.curr.load(1, Ordering::AcqRel) % cap;

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
                    self.curr.store(pos, Ordering::Release);

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

    /// Try to return an element to the `SyncPool`. If succeed, we will return `None` to indicate that
    /// the value has been placed in an empty slot; otherwise, we will return `Option<Box<T>>` such
    /// that the caller can decide if the element shall be just discarded, or try put it back again.
    pub fn put(&mut self, val: Box<T>) -> Option<Box<T>> {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter);

        // start from where we're left
        let cap = self.slots.len();
        let mut trials = 2 * cap;
        let mut pos: usize = self.curr.load(1, Ordering::Acquire) % cap;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the access or move on
            if let Ok(i) = slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.store(pos, Ordering::Release);

                // put the value back and reset
                slot.release(i, val, self.reset_handle);
                slot.leave(i as u16);

                return None;
            }

/*            if slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.store(pos, Ordering::Release);

                // put the value back into the slot
                slot.release(val, self.reset_handle.load(Ordering::Acquire));
                slot.leave();

                return true;
            }*/

            // hold off a bit to reduce contentions
            cpu_relax(SPIN_PERIOD / 2);

            // update states
            pos = self.curr.fetch_sub(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                return Some(val);
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
            reset_handle: None,
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

        // now drop the reset handle if it's not null
        self.reset_handle.take();
    }
}

pub trait PoolState {
    fn expansion_enabled(&self) -> bool;

    fn miss_count(&self) -> usize;

    fn capacity(&self) -> usize;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Default> PoolState for SyncPool<T> {
    fn expansion_enabled(&self) -> bool {
        let configure = self.configure.load(Ordering::SeqCst);
        configure & CONFIG_ALLOW_EXPANSION > 0
    }

    fn miss_count(&self) -> usize {
        self.miss_count.load(Ordering::Acquire)
    }

    fn capacity(&self) -> usize {
        self.slots.len() * SLOT_CAP
    }

    fn len(&self) -> usize {
        self.slots.iter().fold(0, |sum, item| sum + item.size_hint())
    }
}

pub trait PoolManager<T> {
    fn reset_handle(&mut self, handle: fn(&mut T)) -> &mut Self;
    fn allow_expansion(&mut self, allow: bool) -> &mut Self;
    fn expand(&mut self, additional: usize, block: bool) -> bool;
    fn refill(&mut self, count: usize) -> usize;
}

/// The pool manager that provide many useful utilities to keep the SyncPool close to the needs of
/// the caller program.
impl<T: Default> PoolManager<T> for SyncPool<T>
where
    T: Default,
{
    /// Set or update the reset handle. If set, the reset handle will be invoked every time an element
    /// has been returned back to the pool (i.e. calling the `put` method), regardless of if the element
    /// is created by the pool or not.
    fn reset_handle(&mut self, handle: fn(&mut T)) -> &mut Self {
        // busy waiting ... for the first chance a barrier owned by someone else is lowered
        let mut count: usize = 8;
        let timeout = Instant::now().add(Duration::from_millis(16));

        loop {
            match self
                .visitor_counter
                .1
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(_) => {
                    cpu_relax(count);

                    if count > 4 {
                        // update the counter (and the busy wait period)
                        count -= 1;
                    } else if Instant::now() > timeout {
                        // don't block for more than 16ms
                        return self;
                    }
                }
            }
        };

        self.reset_handle
            .replace(handle);

        self.visitor_counter.1.store(false, Ordering::SeqCst);
        self
    }

    /// Set or update the settings that if we will allow the `SyncPool` to be expanded.
    fn allow_expansion(&mut self, allow: bool) -> &mut Self {
        if !(self.expansion_enabled() ^ allow) {
            // not flipping the configuration, return
            return self;
        }

        self.update_config(CONFIG_ALLOW_EXPANSION, allow);
        self
    }

    /// Try to expand the `SyncPool` and add more elements to it. Usually invoke this API only when
    /// the caller is certain that the pool is under pressure, and that a short block to the access
    /// of the pool won't cause serious issues, since the function will block the current caller's
    /// thread until it's finished (i.e. get the opportunity to raise the writer's barrier and wait
    /// everyone to leave).
    ///
    /// If we're unable to expand the pool, it's due to one of the following reasons: 1) someone has
    /// already raised the writer's barrier and is likely modifying the pool, we will leave immediately,
    /// and it's up to the caller if they want to try again; 2) we've waited too long but still couldn't
    /// obtain an exclusive access to the pool, and similar to reason 1), we will quit now.
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
        let mut count: usize = 8;
        let safe = loop {
            match self
                .visitor_counter
                .0
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::Relaxed)
            {
                Ok(_) => break true,
                Err(_) => {
                    cpu_relax(2);

                    if count > 2 {
                        count -= 1;
                    } else if !block {
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

    /// Due to contentious access to the pool, sometimes the `put` action could not finish and return
    /// the element to the pool successfully. Overtime, this could cause the number of elements in the
    /// pool to dwell. This would only happen slowly if we're running a very contentious multithreading
    /// program, but it surely could happen. If the caller detects such situation, they can invoke the
    /// `refill` API and try to refill the pool with elements.
    ///
    /// We will try to refill as many elements as requested
    fn refill(&mut self, additional: usize) -> usize {
        let cap = self.capacity();
        let empty_slots = cap - self.len();

        if empty_slots == 0 {
            return 0;
        }

        let quota = if additional > empty_slots {
            empty_slots
        } else {
            additional
        };

        let mut count = 0;
        let timeout = Instant::now().add(Duration::from_millis(16));

        // try to put `quota` number of elements into the pool
        while count < quota {
            let mut val = Box::new(Default::default());
            let mut runs = 0;

            // retry to put the allocated element into the pool.
            while let Some(ret) = self.put(val) {
                val = ret;
                runs += 1;

                // timeout
                if Instant::now() > timeout {
                    return count;
                }

                // check the pool length for every 4 failed attempts to put the element into the pool.
                if runs % 4 == 0 && self.len() == cap {
                    return count;
                }

                // relax a bit
                cpu_relax(if runs < 16 { runs / 2 } else { 8 });
            }

            count += 1;
        }

        count
    }
}
