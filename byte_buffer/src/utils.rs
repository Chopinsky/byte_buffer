use std::mem;
use std::vec;

pub(crate) enum BufOp {
    Reserve(bool),
    Release(usize, bool),
    ReleaseAndExtend(Vec<u8>, bool),
    Extend(usize),
}

pub(crate) enum WorkerOp {
    Cleanup(usize, bool),
    Shutdown,
}

pub(crate) enum SliceStatusQuery {
    Length,
    Capacity,
}

pub(crate) fn make_buffer(cap: usize) -> *mut u8 {
    let mut v: Vec<u8> = vec::from_elem(0, cap);
    let p = v.as_mut_ptr();
    mem::forget(v);
    p
}
