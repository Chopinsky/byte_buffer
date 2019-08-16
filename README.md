## The sync data libraries
This repository contains a few crates that help `Rust` programs to handle heavily used
data elements more gracefully, and hence improve the overall performance around hot
code region. 

In particular, the repo currently have 2 published crates:
* [`byte_buffer`](): a library for reusing byte array in heavy I/O code;
* `sync_pool`: a library that's more generic for reusing heavy and (usually) heap based
data elements.   