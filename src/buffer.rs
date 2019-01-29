use std::str;
use std::vec;
use crate::manager;

pub struct ByteBuffer {
    buf: Vec<u8>,
    is_lent: bool,
}

pub trait BufferOp {
    fn as_writable(&mut self) -> &mut Vec<u8>;
    fn as_writable_slice(&mut self) -> &mut [u8];
    fn read(&self) -> &Vec<u8>;
    fn read_as_slice(&self) -> &[u8];
    fn swap(&mut self, buffer: Vec<u8>) -> Vec<u8>;
    fn take(&mut self) -> Vec<u8>;
    fn reset(&mut self);
    fn try_into_string(&self) -> Result<String, String>;
}

impl ByteBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        ByteBuffer {
            buf: vec::from_elem(0, capacity),
            is_lent: false,
        }
    }

    pub(crate) fn update_status(&mut self, is_lent: bool) {
        self.is_lent = is_lent;
    }

    // Super unsafe, as we're using super unsafe [`Vec::from_raw_parts`] here... Swap out the inner
    // buf and replace it with a new Vec<u8>. The swapped out buf will transfer the ownership to
    // the caller of this function.
    fn buf_swap(&mut self, target: Vec<u8>) -> Vec<u8> {
        let res = unsafe {
            Vec::from_raw_parts(self.buf.as_mut_ptr(), self.buf.len(), self.buf.capacity())
        };

        self.buf = target;
        res
    }
}

impl BufferOp for ByteBuffer {
    fn as_writable(&mut self) -> &mut Vec<u8> {
        &mut self.buf
    }

    fn as_writable_slice(&mut self) -> &mut [u8] {
        self.buf.as_mut_slice()
    }

    fn read(&self) -> &Vec<u8> {
        &self.buf
    }

    fn read_as_slice(&self) -> &[u8] {
        &self.buf.as_slice()
    }

    fn swap(&mut self, buffer: Vec<u8>) -> Vec<u8> {
        self.buf_swap(buffer)
    }

    fn take(&mut self) -> Vec<u8> {
        self.buf_swap(vec::from_elem(0, manager::buffer_capacity()))
    }

    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    fn try_into_string(&self) -> Result<String, String> {
        match str::from_utf8(&self.buf.as_slice()) {
            Ok(raw) => Ok(String::from(raw)),
            Err(e) => Err(format!(
                "Unable to convert the buffered data into utf-8 string, error occurs at {}",
                e.valid_up_to()
            )),
        }
    }
}

impl Drop for ByteBuffer {
    fn drop(&mut self) {
        // if buffer is dropped without being released back to the buffer pool, try save it.
        if self.is_lent {
            // swap the pointer out so it won't be killed by drop
            let vec = self.buf_swap(Vec::new());

            manager::push_back(ByteBuffer {
                buf: vec,
                is_lent: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
