//! This crate provides an easy-to-use and manageable byte buffer in frequent I/O operations, where
//! implementations of the Read/Write traits are used extensively. With our pre-allocated buffer
//! pool, your I/O code can save tons of wasted CPU cycles for dynamic buffer allocations, which are
//! often the bottleneck of the throughout performance.
//!
//! To use this crate, add the crate dependency in your project's Cargo.toml file:
//!
//! ```
//! [dependencies]
//! byte_buffer = "0.1"
//! ```
//!
//! # Examples
//!
//! ```
//! extern crate byte_buffer;
//! use byte_buffer::prelude::*;
//!
//! fn main() {
//!   // Count of buffer: 10; Buffer capacity: 3
//!   ByteBuffer::init(10, 3);
//!
//!   // Slice the buffer for use in your code
//!   let mut buffer = ByteBuffer::slice();
//!
//!   // Fill the buffer with some byte data
//!   io::repeat(0b101).read_exact(buffer.as_writable().unwrap()).unwrap();
//!
//!   // Read the data out. The buffer will be released back to the pool after going out of the scope
//!   assert_eq!(buffer.as_readable().unwrap(), [0b101, 0b101, 0b101]);
//! }
//! ```

use crossbeam_channel as channel;

pub mod manager;
mod buffer;
mod lock;

pub mod prelude {
    pub use crate::manager::*;
}

#[macro_export]
macro_rules! slice_buffer {
    () => {{
        crate::manager::slice()
    }};
}

#[macro_export]
macro_rules! try_slice {
    () => {{
        crate::manager::try_slice()
    }};
}