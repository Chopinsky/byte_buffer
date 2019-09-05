extern crate byte_buffer;

use byte_buffer::prelude::ByteBuffer;
use std::io::{self, Read};

fn main() {
    ByteBuffer::init(10, 3);

    let mut buffer = ByteBuffer::slice();
    io::repeat(0b101).read_exact(buffer.as_writable()).unwrap();

    println!("Slice content: {:?}", buffer.read());
    assert_eq!(buffer.read().unwrap(), [0b101, 0b101, 0b101]);
}
