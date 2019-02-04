use crossbeam_channel as channel;

pub mod manager;
mod buffer;
mod lock;

pub mod prelude {
//    pub use crate::buffer::{ByteBuffer, BufferOp};
//    pub use crate::manager::{init, reserve, try_reserve, release};
}

#[macro_export]
macro_rules! reserve {
    () => {{
//        crate::manager::reserve()
    }};
}

#[macro_export]
macro_rules! try_reserve {
    () => {{
//        crate::manager::try_reserve()
    }};
}

#[macro_export]
macro_rules! release {
    ($x:ident) => {{
//        crate::manager::release($x);
    }};
}