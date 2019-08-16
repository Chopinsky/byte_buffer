
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::sync::{
    mpsc::{SyncSender, Receiver, TrySendError},
};

pub(crate) enum Message<T> {
    Close,
    Release(*mut ManuallyDrop<Box<T>>),
}

#[derive(Default)]
pub struct Host<T>
    where T: Default
{
    val: ManuallyDrop<Box<T>>,
    chan: Option<SyncSender<Message<T>>>,
}

impl<T> Deref for Host<T>
    where T: Default
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl<T> DerefMut for Host<T>
    where T: Default
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.val
    }
}

impl<T> Drop for Host<T>
    where T: Default
{
    fn drop(&mut self) {
        if self.chan.is_none() {
            unsafe { ManuallyDrop::drop(&mut self.val); }
            return;
        }

        self.chan
            .as_ref()
            .unwrap()
            .try_send(Message::Release(&mut self.val as *mut ManuallyDrop<Box<T>>))
            .map_err(|err| {
                // extract the content
                let msg = match err {
                    TrySendError::Full(m) => m,
                    TrySendError::Disconnected(m) => m,
                };

                // failed to return the value, must manually drop the value now
                if let Message::Release(ptr) = msg {
                    unsafe { ManuallyDrop::drop(&mut *ptr); }
                };
            })
            .unwrap_or_default();
    }
}