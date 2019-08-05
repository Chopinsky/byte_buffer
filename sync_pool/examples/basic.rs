extern crate sync_pool;

use std::time::Duration;
use std::thread;
use std::sync::mpsc;
use sync_pool::prelude::*;

const ARR_CAP: usize = 32;

static mut POOL: Option<SyncPool<[u8; ARR_CAP]>> = None;

fn main() {
    unsafe {
        let mut pool = SyncPool::with_size(64);
        pool.allow_expansion(true);
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

    let (tx, rx) = mpsc::sync_channel(16);

    let handle_one = thread::spawn(move || {
        thread::sleep(Duration::from_micros(10));

        for _ in 0..32 {
            let arr = unsafe { POOL.as_mut().unwrap().get() };
            assert_eq!(arr.len(), 32);

            tx.send(arr).unwrap_or_default();
        }

        println!("Child thread one done...");
    });

    let handle_two = thread::spawn(move || {
        thread::sleep(Duration::from_micros(5));

        while let Ok(arr) = rx.recv() {
            assert_eq!(arr.len(), 32);

            unsafe { POOL.as_mut().unwrap().put(arr); }
        }

        println!("Child thread two done...");
    });

    for _ in 0..32 {
        let arr = unsafe { POOL.as_mut().unwrap().get() };
        assert_eq!(arr.len(), 32);
    }

    println!("Main thread done...");

    handle_one.join().unwrap_or_default();
    handle_two.join().unwrap_or_default();
}

fn cleaner(slice: &mut [u8; ARR_CAP]) {
    for i in 0..slice.len() {
        slice[i] = 0;
    }

    println!("Byte slice cleared...");
}