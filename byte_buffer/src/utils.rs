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