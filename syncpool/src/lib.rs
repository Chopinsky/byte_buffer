mod bucket;
mod pool;
mod utils;

pub use crate::pool::{PoolManager, PoolState, SyncPool};

pub mod prelude {
    pub use crate::{PoolManager, PoolState, SyncPool};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check() {
        let mut pool: SyncPool<[u8; 32]> = SyncPool::with_size(12);

        for _ in 0..32 {
            let ary = pool.get();
            assert_eq!(ary.len(), 32);
            pool.put(ary);
        }
    }
}
