use crate::bucket::*;
use crate::utils::{cpu_relax, make_elem};
use std::ops::Add;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const POOL_SIZE: usize = 8;
const EXPANSION_CAP: usize = 512;
const SPIN_PERIOD: usize = 4;

/// Configuration flag (@ bit positions):
/// 1 -> If the pool is allowed to expand when under pressure
const CONFIG_ALLOW_EXPANSION: usize = 1;

pub(crate) enum ElemBuilder<T> {
    Default(fn() -> Box<T>),
    Builder(fn() -> T),
    Packer(fn(Box<T>) -> Box<T>),
}

struct VisitorGuard<'a>(&'a AtomicUsize);

impl<'a> VisitorGuard<'a> {
    fn register(base: &'a (AtomicUsize, AtomicBool), get: bool) -> Option<Self> {
        let mut count = 8;

        // wait if the underlying storage is in protection mode
        while base.1.load(Ordering::Relaxed) {
            if get {
                return None;
            }

            cpu_relax(count);

            if count > 4 {
                count -= 1;
            }
        }

        base.0.fetch_add(1, Ordering::AcqRel);

        Some(VisitorGuard(&base.0))
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
    curr: (AtomicUsize, AtomicUsize),

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

    /// The builder that will be tasked to create a new instance of the data when the pool is unable
    /// to render one.
    builder: ElemBuilder<T>,
}

impl<T: Default> SyncPool<T> {
    /// Create a pool with default size of 64 pre-allocated elements in it.
    pub fn new() -> Self {
        Self::make_pool(POOL_SIZE, ElemBuilder::Default(Default::default))
    }

    /// Create a `SyncPool` with pre-defined number of elements. Note that we will round-up
    /// the size such that the total number of elements in the pool will mod to 8.
    pub fn with_size(size: usize) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size, ElemBuilder::Default(Default::default))
    }
}

impl<T> SyncPool<T> {
    /// Create a pool with default size of 64 pre-allocated elements in it, which will use the `builder`
    /// handler to obtain the initialized instance of the struct, and then place the object into the
    /// `syncpool` for later use.
    ///
    /// Note that the handler shall be responsible for creating and initializing the struct object
    /// with all fields being valid. After all, they will be the same objects provided to the caller
    /// when invoking the `get` call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use syncpool::*;
    /// use std::vec;
    ///
    /// struct BigStruct {
    ///     a: u32,
    ///     b: u32,
    ///     c: Vec<u8>,
    /// }
    ///
    /// let mut pool = SyncPool::with_builder(|| {
    ///     BigStruct {
    ///         a: 1,
    ///         b: 42,
    ///         c: vec::from_elem(0u8, 0x1_000_000),
    ///     }
    /// });
    ///
    /// let big_box: Box<BigStruct> = pool.get();
    ///
    /// assert_eq!(big_box.a, 1);
    /// assert_eq!(big_box.b, 42);
    /// assert_eq!(big_box.c.len(), 0x1_000_000);
    ///
    /// pool.put(big_box);
    /// ```
    pub fn with_builder(builder: fn() -> T) -> Self {
        Self::make_pool(POOL_SIZE, ElemBuilder::Builder(builder))
    }

    /// Create a `SyncPool` with pre-defined number of elements and a packer handler. The `builder`
    /// handler shall essentially function the same way as in the `with_builder`, that it shall take
    /// the responsibility to create and initialize the element, and return the instance at the end
    /// of the `builder` closure. Note that we will round-up the size such that the total number of
    /// elements in the pool will mod to 8.
    pub fn with_builder_and_size(size: usize, builder: fn() -> T) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size, ElemBuilder::Builder(builder))
    }

    /// Create a pool with default size of 64 pre-allocated elements in it, which will use the `packer`
    /// handler to initialize the element that's being provided by the pool.
    ///
    /// Note that the handler shall take a boxed instance of the element that only contains
    /// placeholder fields, and it is the caller/handler's job to initialize the fields and pack it
    /// with valid and meaningful values. If the struct is valid with all-zero values, the handler
    /// can just return the input element.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use syncpool::*;
    /// use std::vec;
    ///
    /// struct BigStruct {
    ///     a: u32,
    ///     b: u32,
    ///     c: Vec<u8>,
    /// }
    ///
    /// let mut pool = SyncPool::with_packer(|mut src: Box<BigStruct>| {
    ///     src.a = 1;
    ///     src.b = 42;
    ///     src.c = vec::from_elem(0u8, 0x1_000_000);
    ///     src
    /// });
    ///
    /// let big_box: Box<BigStruct> = pool.get();
    ///
    /// assert_eq!(big_box.a, 1);
    /// assert_eq!(big_box.b, 42);
    /// assert_eq!(big_box.c.len(), 0x1_000_000);
    ///
    /// pool.put(big_box);
    /// ```
    pub fn with_packer(packer: fn(Box<T>) -> Box<T>) -> Self {
        Self::make_pool(POOL_SIZE, ElemBuilder::Packer(packer))
    }

    /// Create a `SyncPool` with pre-defined number of elements and a packer handler. The `packer`
    /// handler shall essentially function the same way as in `with_packer`, that it shall take the
    /// responsibility to initialize all the fields of a placeholder struct on the heap, otherwise
    /// the element returned by the pool will be essentially undefined, unless all the struct's
    /// fields can be represented by a 0 value. In addition, we will round-up the size such that
    /// the total number of elements in the pool will mod to 8.
    pub fn with_packer_and_size(size: usize, packer: fn(Box<T>) -> Box<T>) -> Self {
        let mut pool_size = size / SLOT_CAP;
        if pool_size < 1 {
            pool_size = 1
        }

        Self::make_pool(pool_size, ElemBuilder::Packer(packer))
    }

    /// Try to obtain a pre-allocated element from the pool. This method will always succeed even if
    /// the pool is empty or not available for anyone to access, and in this case, a new boxed-element
    /// will be created.
    pub fn get(&mut self) -> Box<T> {
        // update user count
        let guard = VisitorGuard::register(&self.visitor_counter, true);
        if guard.is_none() {
            return make_elem(&self.builder);
        }

        // start from where we're left
        let cap = self.slots.len();
        let mut trials = cap;
        let mut pos: usize = self.curr.0.load(Ordering::Acquire) % cap;

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
                    self.curr.0.store(pos, Ordering::Release);

                    // done
                    return val;
                }

                // failed to checkout, break and let the remainder logic to handle the rest
                break;
            }

            // hold off a bit to reduce contentions
            cpu_relax(SPIN_PERIOD);

            // update to the next position now.
            pos = self.curr.0.fetch_add(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                break;
            }
        }

        // make sure our guard has been returned if we want the correct visitor count
        drop(guard);
        self.miss_count.fetch_add(1, Ordering::Relaxed);

        // create a new object
        make_elem(&self.builder)
    }

    /// Try to return an element to the `SyncPool`. If succeed, we will return `None` to indicate that
    /// the value has been placed in an empty slot; otherwise, we will return `Option<Box<T>>` such
    /// that the caller can decide if the element shall be just discarded, or try put it back again.
    pub fn put(&mut self, val: Box<T>) -> Option<Box<T>> {
        // update user count
        let _guard = VisitorGuard::register(&self.visitor_counter, false);

        // start from where we're left
        let cap = self.slots.len();
        let mut trials = 2 * cap;
        let mut pos: usize = self.curr.1.load(Ordering::Acquire) % cap;

        loop {
            // check this slot
            let slot = &mut self.slots[pos];

            // try the access or move on
            if let Ok(i) = slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.1.store(pos, Ordering::Release);

                // put the value back and reset
                slot.release(i, val, self.reset_handle);
                slot.leave(i as u16);

                return None;
            }

            /*            if slot.access(false) {
                // now we're locked, get the val and update internal states
                self.curr.1.store(pos, Ordering::Release);

                // put the value back into the slot
                slot.release(val, self.reset_handle.load(Ordering::Acquire));
                slot.leave();

                return true;
            }*/

            // hold off a bit to reduce contentions
            if trials < cap {
                cpu_relax(SPIN_PERIOD);
            } else {
                thread::yield_now();
            }

            // update states
            pos = self.curr.1.fetch_add(1, Ordering::AcqRel) % cap;
            trials -= 1;

            // we've finished 1 loop but not finding a value to extract, quit
            if trials == 0 {
                return Some(val);
            }
        }
    }

    fn make_pool(size: usize, builder: ElemBuilder<T>) -> Self {
        let mut pool = SyncPool {
            slots: Vec::with_capacity(size),
            curr: (AtomicUsize::new(0), AtomicUsize::new(0)),
            visitor_counter: (AtomicUsize::new(1), AtomicBool::new(false)),
            miss_count: AtomicUsize::new(0),
            configure: AtomicUsize::new(0),
            reset_handle: None,
            builder,
        };

        pool.add_slots(size, true);
        pool
    }

    #[inline]
    fn add_slots(&mut self, count: usize, fill: bool) {
        let filler = if fill { Some(&self.builder) } else { None };

        for _ in 0..count {
            // self.slots.push(Bucket::new(fill));
            self.slots.push(Bucket2::new(filler));
        }
    }

    fn update_config(&mut self, mask: usize, target: bool) {
        let mut config = self.configure.load(Ordering::SeqCst);

        while let Err(old) = self.configure.compare_exchange(
            config,
            config ^ mask,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) {
            if !((old & mask > 0) ^ target) {
                // the configure already matches, we're done
                return;
            }

            config = old;
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

impl<T> PoolState for SyncPool<T> {
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
        self.slots
            .iter()
            .fold(0, |sum, item| sum + item.size_hint())
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
impl<T> PoolManager<T> for SyncPool<T> {
    /// Set or update the reset handle. If set, the reset handle will be invoked every time an element
    /// has been returned back to the pool (i.e. calling the `put` method), regardless of if the element
    /// is created by the pool or not.
    fn reset_handle(&mut self, handle: fn(&mut T)) -> &mut Self {
        // busy waiting ... for the first chance a barrier owned by someone else is lowered
        let mut count: usize = 8;
        let timeout = Instant::now().add(Duration::from_millis(16));

        loop {
            match self.visitor_counter.1.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => {
                    cpu_relax(count);

                    // update the counter (and the busy wait period)
                    count -= 1;

                    if count < 4 {
                        // yield the thread for later try
                        thread::yield_now();
                    } else if Instant::now() > timeout {
                        // don't block for more than 16ms
                        return self;
                    }
                }
            }
        }

        self.reset_handle.replace(handle);

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
                    count -= 1;

                    if count < 4 {
                        thread::yield_now();
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
            let mut val = make_elem(&self.builder);
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
                if runs > 8 {
                    thread::yield_now();
                } else {
                    cpu_relax(runs / 2);
                }
            }

            count += 1;
        }

        count
    }
}

#[cfg(test)]
mod pool_tests {
    use super::*;
    use std::vec;

    struct BigStruct {
        a: u32,
        b: u32,
        c: Vec<u8>,
    }

    impl BigStruct {
        fn new() -> Self {
            BigStruct {
                a: 1,
                b: 42,
                c: vec::from_elem(0u8, 0x1_000_000),
            }
        }

        fn initializer(mut self: Box<Self>) -> Box<Self> {
            self.a = 1;
            self.b = 42;
            self.c = vec::from_elem(0u8, 0x1_000_000);

            self
        }
    }

    #[test]
    fn use_packer() {
        let mut pool = SyncPool::with_packer(BigStruct::initializer);

        let big_box = pool.get();

        assert_eq!(big_box.a, 1);
        assert_eq!(big_box.b, 42);
        assert_eq!(big_box.c.len(), 0x1_000_000);
    }

    #[test]
    fn use_builder() {
        let mut pool = SyncPool::with_builder(BigStruct::new);

        let big_box = pool.get();

        assert_eq!(big_box.a, 1);
        assert_eq!(big_box.b, 42);
        assert_eq!(big_box.c.len(), 0x1_000_000);
    }
}
