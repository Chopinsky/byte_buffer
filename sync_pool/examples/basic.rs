extern crate sync_pool;

use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::sync::mpsc;
use std::sync::mpsc::SyncSender;
use std::thread;
use std::time::Duration;
use sync_pool::prelude::*;

/// Number of producers that runs in this test
const COUNT: usize = 128;

/// A shared pool, one can imagine other ways of sharing the pool concurrently, here we choose to use
/// an unsafe version to simplify the example.
static mut POOL: MaybeUninit<SyncPool<Box<ComplexStruct>>> = MaybeUninit::uninit();

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

/// Make sure we build up the pool before use
unsafe fn pool_setup() -> (
    Pin<&'static mut SyncPool<Box<ComplexStruct>>>,
    Pin<&'static mut SyncPool<Box<ComplexStruct>>>,
) {
    POOL.as_mut_ptr().write(SyncPool::with_size(COUNT / 2));

    (
        Pin::new(&mut *POOL.as_mut_ptr()),
        Pin::new(&mut *POOL.as_mut_ptr()),
    )
}

/// Main example body
fn main() {
    // let's make the pool slightly smaller than the demand, this will simulate a service under pressure
    // such that the pool can't completely meet the demand without dynamically expand the pool.
    let (pinned_producer, pinned_consumer) = unsafe { pool_setup() };

    // make the channel that establish a concurrent pipeline.
    let (tx, rx) = mpsc::sync_channel(64);

    // data producer loop
    thread::spawn(move || {
        let producer = pinned_producer.get_mut();

        for i in 0..COUNT {
            run(producer, &tx, i);
        }
    });

    // data consumer logic
    let handler = thread::spawn(move || {
        let consumer = pinned_consumer.get_mut();

        for content in rx {
            println!("Receiving struct with id: {}", content.id);
            consumer.put(content);
        }
    });

    // wait for the receiver to finish and print the result.
    handler.join().unwrap_or_default();

    println!("All done...");
}

fn run(pool: &mut SyncPool<Box<ComplexStruct>>, chan: &SyncSender<Box<ComplexStruct>>, id: usize) {
    // take a pre-init struct from the pool
    let mut content = pool.get();
    content.id = id;

    // assuming we're doing some stuff in this period
    thread::sleep(Duration::from_nanos(32));

    // done with the stuff, send the result out.
    chan.send(content).unwrap_or_default();
}
