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
//! Then declare the use of the crate:
//!
//! ```
//! extern crate byte_buffer;
//! use byte_buffer::prelude::*;
//! ```
//!
//! # Examples
//!
//! Before use, you need to define the size of the buffer and the capacity of each
//! buffer slice you would like to acquire in the code.
//!
//! ```
//! let v: Vec<i32> = Vec::new();
//! ```
//!
//! ...or by using the [`vec!`] macro:
//!
//! ```
//! let v: Vec<i32> = vec![];
//!
//! let v = vec![1, 2, 3, 4, 5];
//!
//! let v = vec![0; 10]; // ten zeroes
//! ```
//!
//! You can [`push`] values onto the end of a vector (which will grow the vector
//! as needed):
//!
//! ```
//! let mut v = vec![1, 2];
//!
//! v.push(3);
//! ```
//!
//! Popping values works in much the same way:
//!
//! ```
//! let mut v = vec![1, 2];
//!
//! let two = v.pop();
//! ```
//!
//! Vectors also support indexing (through the [`Index`] and [`IndexMut`] traits):
//!
//! ```
//! let mut v = vec![1, 2, 3];
//! let three = v[2];
//! v[1] = v[1] + 5;
//! ```

use crossbeam_channel as channel;

pub mod manager;
mod buffer;
mod lock;

pub mod prelude {
    pub use crate::manager::*;
}

#[macro_export]
macro_rules! slice {
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

#[macro_export]
macro_rules! release {
    ($x:ident) => {{
//        crate::manager::release($x);
    }};
}