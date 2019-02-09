[Byte_Buffer][docsrs]
======================

[![Rusty_Express on crates.io][cratesio-image]][cratesio]
[![Rusty_Express on docs.rs][docsrs-image]][docsrs]

[cratesio]: https://crates.io/crates/byte_buffer
[cratesio-image]: https://img.shields.io/crates/v/byte_buffer.svg
[docsrs-image]: https://docs.rs/byte_buffer/badge.svg
[docsrs]: https://docs.rs/byte_buffer

## What is this
This crate provides an easy-to-use and manageable byte buffer in frequent I/O operations, where
implementations of the Read/Write traits are used extensively. With our pre-allocated buffer 
pool, your I/O code can save tons of wasted CPU cycles for dynamic buffer allocations, which are 
often the bottleneck of the throughout performance.

## Use this crate
To use this crate, add the crate dependency in your project's Cargo.toml file:

```
[dependencies]
byte_buffer = "0.1"
```

Then declare the use of the crate:

```rust
extern crate byte_buffer;
use byte_buffer::prelude::*;
```

## Contributions are welcome!
Please feel free to submit bug reports or features.