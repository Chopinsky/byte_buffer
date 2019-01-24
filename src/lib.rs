pub mod buffer;
pub mod manager;

pub mod prelude {
    pub use crate::buffer::{ByteBuffer, BufferOp};
    pub use crate::manager::{init, reserve};
}
