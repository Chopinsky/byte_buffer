#![allow(dead_code)]

extern crate sync_pool;

use std::time::{Duration, Instant};
use std::thread;
use std::sync::mpsc;
use sync_pool::prelude::*;

const ARR_CAP: usize = 4096;
const TEST_SIZE: usize = 10240;

static mut POOL: Option<SyncPool<Buffer>> = None;

struct Buffer(Box<[u8; ARR_CAP]>);

impl Buffer {
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer(Box::new([0u8; ARR_CAP]))
//        Buffer([0u8; ARR_CAP])
    }
}

fn main() {
    native();
    pool();
}

fn native() {
    let (tx, rx) = mpsc::sync_channel(32);
    let tx_clone = tx.clone();

    let now = Instant::now();

    let send_one = thread::spawn(move || {
        thread::sleep(Duration::from_micros(10));

        for _ in 0..TEST_SIZE {
            let arr: Buffer = Default::default();
            assert_eq!(arr.len(), ARR_CAP);

            tx_clone.send(arr).unwrap_or_default();
        }

        println!("Child thread oneA done...");
    });

    let send_two = thread::spawn(move || {
        thread::sleep(Duration::from_micros(10));

        for _ in 0..TEST_SIZE {
            let arr: Buffer = Default::default();
            assert_eq!(arr.len(), ARR_CAP);

            tx.send(arr).unwrap_or_default();
        }

        println!("Child thread oneB done...");
    });

    let recv_one = thread::spawn(move || {
        thread::sleep(Duration::from_micros(5));

        while let Ok(arr) = rx.recv() {
            assert_eq!(arr.len(), ARR_CAP);
        }

        println!("Child thread two done...");
    });

    for i in 0..TEST_SIZE {
        // sleep a bit to create some concurrent actions
        if i % 101 == 1 {
            thread::sleep(Duration::from_micros(1));
        }

        let arr: Buffer = Default::default();
        assert_eq!(arr.len(), ARR_CAP);
    }

    println!("Main thread done...");

    send_one.join().unwrap_or_default();
    send_two.join().unwrap_or_default();
    recv_one.join().unwrap_or_default();

    println!("Native: {}", now.elapsed().as_millis());
}

fn pool() {
    unsafe {
        let mut pool: SyncPool<Buffer> = SyncPool::with_size(64);
        pool.reset_handle(cleaner);

    /*
        // Alternatively, use an anonymous function for the same purpose. Closure can't be used as
        // a handle, though.
        pool.reset_handle(|slice: &mut [u8; ARR_CAP]| {
            for i in 0..slice.len() {
                slice[i] = 0;
            }

            println!("Byte slice cleared...");
        });
    */

        POOL.replace(pool);
    }

    let (tx, rx) = mpsc::sync_channel(32);
    let tx_clone = tx.clone();

    let now = Instant::now();

    let send_one = thread::spawn(move || {
        thread::sleep(Duration::from_micros(10));

        for _ in 0..TEST_SIZE {
            let arr = unsafe { POOL.as_mut().unwrap().get() };
            assert_eq!(arr.len(), ARR_CAP);

            tx_clone.try_send(arr).unwrap_or_default();
        }

        println!("Child thread one done...");
    });

    let send_two = thread::spawn(move || {
        thread::sleep(Duration::from_micros(10));

        for _ in 0..TEST_SIZE {
            let arr = unsafe { POOL.as_mut().unwrap().get() };
            assert_eq!(arr.len(), ARR_CAP);

            tx.try_send(arr).unwrap_or_default();
        }

        println!("Child thread one done...");
    });

    let recv_one = thread::spawn(move || {
        thread::sleep(Duration::from_micros(5));

        while let Ok(arr) = rx.recv_timeout(Duration::from_secs(1)) {
            assert_eq!(arr.len(), ARR_CAP);
            unsafe { POOL.as_mut().unwrap().put(arr); }
        }

        println!("Child thread two done...");
    });

    for i in 0..TEST_SIZE {
        // sleep a bit to create some concurrent actions
        if i % 101 == 1 {
            thread::sleep(Duration::from_micros(1));
        }

        let arr = unsafe { POOL.as_mut().unwrap().get() };
        assert_eq!(arr.len(), ARR_CAP);

        unsafe { POOL.as_mut().unwrap().put(arr) };
    }

    println!("Main thread done...");

//    println!("Fault count: {}", unsafe{ POOL.as_mut().unwrap().fault_count() });

    send_one.join().unwrap_or_default();
    send_two.join().unwrap_or_default();
    recv_one.join().unwrap_or_default();

    println!("Pool: {}", now.elapsed().as_millis());
}

fn cleaner(slice: &mut Buffer) {
    for i in 0..slice.len() {
        slice.0[i] = 0;
    }

//    println!("Byte slice cleared...");
}