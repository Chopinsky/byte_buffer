mod pool;

pub use crate::pool::{SyncPool, PoolManager, PoolState};

pub mod prelude {
    pub use crate::{SyncPool, PoolManager, PoolState};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check() {
        let mut pool: SyncPool<[u8; 32]> = SyncPool::with_size(12);

        for _ in 0..32 {
            let ary = pool.get();
        }

        assert_eq!(2 + 2, 4);
    }
}
