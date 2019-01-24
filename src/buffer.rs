use std::str;
use std::vec;
use crate::manager;

pub struct ByteBuffer {
    inner: Vec<Vec<u8>>,
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
    pub(crate) fn new(buffer: Vec<u8>) -> Self {
        ByteBuffer {
            inner: vec![buffer; 1]
        }
    }
}

impl BufferOp for ByteBuffer {
    fn as_writable(&mut self) -> &mut Vec<u8> {
        &mut self.inner[0]
    }

    fn as_writable_slice(&mut self) -> &mut [u8] {
        self.inner[0].as_mut_slice()
    }

    fn read(&self) -> &Vec<u8> {
        &self.inner[0]
    }

    fn read_as_slice(&self) -> &[u8] {
        assert_eq!(self.inner.len(), 1);
        &self.inner[0].as_slice()
    }

    fn swap(&mut self, buffer: Vec<u8>) -> Vec<u8> {
        let old = self.inner.pop().unwrap();
        self.inner.push(buffer);

        old
    }

    fn take(&mut self) -> Vec<u8> {
        let old = self.inner.pop().unwrap();
        self.inner.push(
            vec::from_elem(0, manager::buffer_capacity())
        );

        old
    }

    fn reset(&mut self) {
        self.inner[0].iter_mut().for_each(|val| {
            *val = 0;
        });
    }

    fn try_into_string(&self) -> Result<String, String> {
        match str::from_utf8(&self.inner[0].as_slice()) {
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
        // reset to 0 first
        self.reset();

        // then move the vec back
        if self.inner.len() == 1 {
            manager::push_back(self.inner.swap_remove(0));
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
