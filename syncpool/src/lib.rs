//! [`SyncPool`]
//! A simple and thread-safe objects pool to reuse heavy objects placed in the heap.
//!
//! ## What this crate is for
//! Inspired by Go's `sync.Pool` module, this crate provides a multithreading-friendly
//! library to recycle and reuse heavy, heap-based objects, such that the overall
//! allocation and memory pressure will be reduced, and hence boosting the performance.
//!
//!
//! ## What this crate is NOT for
//! There is no such thing as the silver bullet when designing a multithreading project,
//! programmer has to judge use cases on a case-by-case base.
//!
//! As shown by a few (hundred) benchmarks we have run, it is quite clear that the
//! library can reliably beat the allocator in the following case:
//!
//! The object is large enough that it makes sense to live in the heap.
//! The clean up operations required to sanitize the written data before putting the
//! element back to the pool is simple and fast to run.
//! The estimation on the maximum number of elements simultaneously checked out
//! during the program run is good enough, i.e. the parallelism is deterministic;
//! otherwise when the pool is starving (i.e. it doesn't have enough elements left to
//! provide), the performance will suffer because we will need to create (and allocate
//! in the heap for) new elements.
//!
//! If your struct is nibble enough to live in the stack without blowing it, or if it's
//! not in middle of the hottest code path, you most likely won't need the library to
//! labor for you, allocators nowadays work quite marvelously, especially on the stack.
//!
//!
//! ## Example
//! ```rust
//! extern crate syncpool;
//!
//! use std::collections::HashMap;
//! use std::sync::mpsc::{self, SyncSender};
//! use std::thread;
//! use std::time::Duration;
//! use syncpool::prelude::*;
//!
//! /// For simplicity and illustration, here we use the most simple but unsafe way to
//! /// define the shared pool: make it static mut. Other safer implementation exists
//! /// but may require some detour depending on the business logic and project structure.
//! static mut POOL: Option<SyncPool<ComplexStruct>> = None;
//!
//! /// Number of producers that runs in this test
//! const COUNT: usize = 128;
//!
//! /// The complex data struct for illustration. Usually such a heavy element could also
//! /// contain other nested struct, and should almost always be placed in the heap. If
//! /// your struct is *not* heavy enough to be living in the heap, you most likely won't
//! /// need this library -- the allocator will work better on the stack. The only requirement
//! /// for the struct is that it has to implement the `Default` trait, which can be derived
//! /// in most cases, or implemented easily.
//! #[derive(Default, Debug)]
//! struct ComplexStruct {
//!     id: usize,
//!     name: String,
//!     body: Vec<String>,
//!     flags: Vec<usize>,
//!     children: Vec<usize>,
//!     index: HashMap<usize, String>,
//!     rev_index: HashMap<String, usize>,
//! }
//!
//! fn main() {
//!    // Must initialize the pool first
//!    unsafe { POOL.replace(SyncPool::with_size(COUNT / 2)); }
//!
//!    // use the channel that create a concurrent pipeline.
//!    let (tx, rx) = mpsc::sync_channel(64);
//!
//!    // data producer loop
//!    thread::spawn(move || {
//!        let mut producer = unsafe { POOL.as_mut().unwrap() };
//!
//!        for i in 0..COUNT {
//!            // take a pre-init element from the pool, we won't allocate in this
//!            // call since the boxed element is already placed in the heap, and
//!            // here we only reuse the one.
//!            let mut content: Box<ComplexStruct> = producer.get();
//!            content.id = i;
//!
//!            // simulating busy/heavy calculations we're doing in this time period,
//!            // usually involving the `content` object.
//!            thread::sleep(Duration::from_nanos(32));
//!
//!            // done with the stuff, send the result out.
//!            tx.send(content).unwrap_or_default();
//!        }
//!    });
//!
//!    // data consumer logic
//!    let handler = thread::spawn(move || {
//!        let mut consumer = unsafe { POOL.as_mut().unwrap() };
//!
//!        // `content` has the type `Box<ComplexStruct>`
//!        for content in rx {
//!            println!("Receiving struct with id: {}", content.id);
//!            consumer.put(content);
//!        }
//!    });
//!
//!    // wait for the receiver to finish and print the result.
//!    handler.join().unwrap_or_default();
//!
//!    println!("All done...");
//!
//! }
//! ```
//!
//! In addition, if you prefer to use a constructor for creating and intializing the element, you
//! may opt to use the `with_builder` API:
//!
//! ```rust
//! use syncpool::prelude::*;
//! use std::vec;
//!
//! struct BigStruct {
//!     a: u32,
//!     b: u32,
//!     c: Vec<u8>
//! }
//!
//! impl BigStruct {
//!     fn new() -> Self {
//!         BigStruct {
//!             a: 1,
//!             b: 42,
//!             c: vec::from_elem(0u8, 0x1_000_000),
//!         }
//!     }
//! }
//!
//! let mut pool = SyncPool::with_builder(BigStruct::new);
//!
//! println!("Pool created...");
//!
//! let big_box = pool.get();
//!
//! assert_eq!(big_box.a, 1);
//! assert_eq!(big_box.b, 42);
//! assert_eq!(big_box.c.len(), 0x1_000_000);
//!
//! pool.put(big_box);
//! ```
//!
//! There are occassions where the struct is too large to fit into the stack (e.g. a buffer data
//! structure), you can instead use the `with_packer` API to intialize the object that has already
//! been created on heap, so you can worry less on making the object and move it to the heap (and
//! with automatic performance boost)!
//!
//! Note that if calling the `with_packer` API, you have to make sure that all fields shall be properly
//! initialized. The provided placeholder object is well-aligned, however, the fields may be undefined
//! if not initialized correctly, e.g. a field of the `NonNull<T>` type, or the `MaybeUninit<T>` type.
//!
//! ```rust
//! use syncpool::prelude::*;
//! use std::vec;
//!
//! struct BigStruct {
//!     a: u32,
//!     b: u32,
//!     c: Vec<u8>
//! }
//!
//! impl BigStruct {
//!     fn initializer(mut self: Box<Self>) -> Box<Self> {
//!         self.a = 1;
//!         self.b = 42;
//!         self.c = vec::from_elem(0u8, 0x1_000_000);
//!         self
//!     }
//! }
//!
//! let mut pool = SyncPool::with_packer(BigStruct::initializer);
//!
//! println!("Pool created...");
//!
//! let big_box = pool.get();
//!
//! assert_eq!(big_box.a, 1);
//! assert_eq!(big_box.b, 42);
//! assert_eq!(big_box.c.len(), 0x1_000_000);
//!
//! pool.put(big_box);
//! ```
//!
//! You can find more complex (i.e. practical) use cases in the [examples](https://github.com/Chopinsky/byte_buffer/tree/master/sync_pool/examples)
//! folder.
//!

mod boxed;
mod bucket;
mod pool;
mod utils;

pub use crate::{
    boxed::{default_box, make_box, raw_box, raw_box_zeroed},
    pool::{PoolManager, PoolState, SyncPool},
};

pub mod prelude {
    pub use crate::boxed::*;
    pub use crate::{PoolManager, PoolState, SyncPool};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check() {
        let mut pool: SyncPool<[u8; 32]> = SyncPool::with_size(12);

        for _ in 0..32 {
            let ary = pool.get();
            assert_eq!(ary.len(), 32);
            pool.put(ary);
        }

        assert!(pool.len() > 0);
    }
}
