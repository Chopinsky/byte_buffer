[Sync_Pool][docsrs]
======================

[![Sync_Pool on crates.io][cratesio-image]][cratesio]
[![Sync_Pool on docs.rs][docsrs-image]][docsrs]

[cratesio]: https://crates.io/crates/sync_pool
[cratesio-image]: https://img.shields.io/crates/v/sync_pool.svg
[docsrs-image]: https://docs.rs/sync_pool/badge.svg
[docsrs]: https://docs.rs/sync_pool

## What this crate is for
Inspired by Go's `sync.Pool` module, this crate provides a multithreading-friendly 
library to recycle and reuse heavy, heap-based objects, such that the overall
allocation and memory pressure will be reduced, and hence boosting the performance. 


## What this is not
THere is no silver bullet when 


## Example
```rust
extern crate sync_pool;

use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::Duration;

use sync_pool::prelude::*;

// For simplicity and illustration, here we use the most simple but unsafe way to 
// define the shared pool: make it static mut. Other safer implementation exists 
// but may require some detour depending on the business logic and project structure.
static mut POOL: Option<SyncPool<ComplexStruct>> = None;

/// Number of producers that runs in this test
const COUNT: usize = 128;

/// The complex data struct for illustration. Usually such a heavy element could also
/// contain other nested struct, and should almost always be placed in the heap. If 
/// your struct is *not* heavy enough to be living in the heap, you most likely won't
/// need this library -- the allocator will work better on the stack. 
#[derive(Default, Debug)]
struct ComplexStruct {
    id: usize,
    name: String,
    body: Vec<String>,
    flags: Vec<usize>,
    children: Vec<usize>,
    index: HashMap<usize, String>,
    rev_index: HashMap<String, usize>,
}

fn main() {
    // Must initialize the pool first
    unsafe { POOL.replace(SyncPool::with_size(COUNT / 2)); }

    // use the channel that create a concurrent pipeline.
    let (tx, rx) = mpsc::sync_channel(64);

    // data producer loop
    thread::spawn(move || {
        let mut producer = unsafe { POOL.as_mut().unwrap() };

        for i in 0..COUNT {
            // take a pre-init element from the pool, we won't allocate in this 
            // call since the boxed element is already placed in the heap, and 
            // here we only reuse the one. 
            let mut content: Box<ComplexStruct> = producer.get();
            content.id = i;
        
            // simulating busy/heavy calculations we're doing in this time period, 
            // usually involving the `content` object.
            thread::sleep(Duration::from_nanos(32));
        
            // done with the stuff, send the result out.
            tx.send(content).unwrap_or_default();
        }
    });

    // data consumer logic
    let handler = thread::spawn(move || {
        let mut consumer = unsafe { POOL.as_mut().unwrap() };
    
        // `content` has the type `Box<ComplexStruct>`
        for content in rx {
            println!("Receiving struct with id: {}", content.id);
            consumer.put(content);
        }
    });

    // wait for the receiver to finish and print the result.
    handler.join().unwrap_or_default();

    println!("All done...");

}
```

You can find more complex (i.e. practical) use cases in the
 [examples](https://github.com/Chopinsky/byte_buffer/tree/master/sync_pool/examples)
folder. 