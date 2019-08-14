#![allow(dead_code)]

extern crate sync_pool;

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use sync_pool::prelude::*;

const BUF_CAP: usize = 1024;
const TEST_SIZE: usize = 128;
const SLEEP: u64 = 64;
const DENOMINATOR: usize = 1;

static mut POOL: Option<SyncPool<ComplexStruct>> = None;

//struct Buffer(Box<[u8; BUF_CAP]>);
struct Buffer {
    id: usize,
    buf: [u8; BUF_CAP],
}

impl Buffer {
    fn len(&self) -> usize {
        self.buf.len()
    }
}

impl Default for Buffer {
    fn default() -> Self {
        let mut base = Buffer {
            id: 0,
            buf: [0u8; BUF_CAP],
        };

        //        let mut base = Buffer(Box::new([0u8; BUF_CAP]));

        base.buf[42] = 42;
        base
    }
}

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
    pool_setup();

    let async_mode = true;
    let trials = 64;
    let mut sum = 0;

    println!("Init len: {}", unsafe { POOL.as_ref().unwrap().len() });

    for i in 0..trials {
        let res = if async_mode {
            let p = thread::spawn(|| run(false));
            let n = thread::spawn(|| run(true));

            let p_time = p.join().unwrap_or_default() as i128;
            let n_time = n.join().unwrap_or_default() as i128;

            n_time - p_time
        } else {
            let n_time = run(true) as i128;
            let p_time = run(false) as i128;

            n_time - p_time
        };

        sum += res;

        println!(">>> Trial: {}; Advance: {} us <<<", i, res);
    }

    println!("Remainder len: {}", unsafe { POOL.as_ref().unwrap().len() });

    println!(
        "\nAverage: {} ms\n",
        (sum as f64) / (trials as f64) / 1000f64
    );
}

fn pool_setup() {
    unsafe {
        let mut pool = SyncPool::with_size(128);

        // clean up the underlying buffer, this handle can also be used to shrink the underlying
        // buffer to save for space, though at a cost of extra overhead for doing that.
        pool.reset_handle(cleaner);

        /*

        // Alternatively, use an anonymous function for the same purpose. Closure can't be used as
        // a handle, though.
        pool.reset_handle(|slice: &mut [u8; BUF_CAP]| {
            for i in 0..slice.len() {
                slice[i] = 0;
            }

            println!("Byte slice cleared...");
        });

        */

        POOL.replace(pool);
    }
}

fn run(alloc: bool) -> u128 {
    let (tx, rx) = mpsc::sync_channel(32);
    let tx_clone = tx.clone();

    let now = Instant::now();

    let send_one = thread::spawn(move || {
        for i in 0..TEST_SIZE {
            if i % DENOMINATOR == 0 {
                thread::sleep(Duration::from_nanos(SLEEP));
            }

            let mut data = if alloc {
                Default::default()
            } else {
                unsafe { POOL.as_mut().unwrap().get() }
            };

            assert!(data.id == 21 || data.id == 0);
            assert_ne!(data.id, 42);
            data.id = 42;

            /*            assert_eq!(arr.len(), BUF_CAP);
            assert_eq!(arr.0[42], 42);*/

            tx_clone.try_send(data).unwrap_or_default();
        }
    });

    let send_two = thread::spawn(move || {
        for i in 0..TEST_SIZE {
            if i % DENOMINATOR == 0 {
                thread::sleep(Duration::from_nanos(SLEEP));
            }

            let mut data = if alloc {
                Default::default()
            } else {
                unsafe { POOL.as_mut().unwrap().get() }
            };

            assert!(data.id == 21 || data.id == 0);
            assert_ne!(data.id, 42);
            data.id = 42;

            /*            assert_eq!(arr.len(), BUF_CAP);
            assert_eq!(arr.0[42], 42);*/

            tx.try_send(data).unwrap_or_default();
        }
    });

    let recv_one = thread::spawn(move || {
        thread::sleep(Duration::from_micros(5));

        while let Ok(arr) = rx.recv() {
            //            assert_eq!(arr.len(), BUF_CAP);

            if !alloc {
                unsafe {
                    POOL.as_mut().unwrap().put(arr);
                }
            }
        }
    });

    for i in 0..TEST_SIZE {
        // sleep a bit to create some concurrent actions
        if i % DENOMINATOR == 1 {
            thread::sleep(Duration::from_nanos(SLEEP));
        }

        let mut data = if alloc {
            Default::default()
        } else {
            unsafe { POOL.as_mut().unwrap().get() }
        };

        assert!(data.id == 21 || data.id == 0);
        assert_ne!(data.id, 42);
        data.id = 42;

        /*        assert_eq!(arr.len(), BUF_CAP);
        assert_eq!(arr.0[42], 42);*/

        if !alloc {
            // when done using the object, make sure to put it back so the pool won't dry up
            unsafe { POOL.as_mut().unwrap().put(data) };
        }
    }

    send_one.join().unwrap_or_default();
    send_two.join().unwrap_or_default();
    recv_one.join().unwrap_or_default();

    now.elapsed().as_micros()
}

fn cleaner(data: &mut ComplexStruct) {
    data.id = 21;
}
