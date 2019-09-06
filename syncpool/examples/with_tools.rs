extern crate syncpool;

use std::vec;
use syncpool::prelude::*;

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

fn main() {
    call_builder();
    call_packer();
}

fn call_builder() {
    let mut pool = SyncPool::with_builder(BigStruct::new);

    println!("Pool created...");

    let big_box = pool.get();

    assert_eq!(big_box.a, 1);
    assert_eq!(big_box.b, 42);
    assert_eq!(big_box.c.len(), 0x1_000_000);

    pool.put(big_box);
}

fn call_packer() {
    let mut pool = SyncPool::with_packer(BigStruct::initializer);

    println!("Pool created...");

    let big_box = pool.get();

    assert_eq!(big_box.a, 1);
    assert_eq!(big_box.b, 42);
    assert_eq!(big_box.c.len(), 0x1_000_000);

    pool.put(big_box);
}