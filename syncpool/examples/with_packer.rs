extern crate syncpool;

use std::vec;
use syncpool::prelude::*;

struct BigStruct {
    a: u32,
    b: u32,
    c: Vec<u8>,
}

fn main() {
    let mut pool = SyncPool::with_packer(|mut src: Box<BigStruct>| {
        src.a = 1;
        src.b = 42;
        src.c = vec::from_elem(0u8, 0x1_000_000);
        src
    });

    println!("Pool created...");

    let big_box = pool.get();

    assert_eq!(big_box.a, 1);
    assert_eq!(big_box.b, 42);
    assert_eq!(big_box.c.len(), 0x1_000_000);

    pool.put(big_box);
}