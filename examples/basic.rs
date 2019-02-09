extern crate byte_buffer;

use std::io::{self, Read};
use byte_buffer::prelude::{ByteBuffer};

fn main() {
    ByteBuffer::init(10, 3);

    let mut buffer = ByteBuffer::slice();
    io::repeat(0b101).read_exact(buffer.as_writable().unwrap()).unwrap();

    println!("Slice content: {:?}", buffer.as_readable().unwrap());
    assert_eq!(buffer.as_readable().unwrap(), [0b101, 0b101, 0b101]);
}