#![allow(unused)]

use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::sync::{
    Arc,
    mpsc::{SyncSender, Receiver, TrySendError},
    atomic::{AtomicPtr, AtomicBool, Ordering},
};

enum Message<T> {
    Shutdown,
    Release(*mut ManuallyDrop<Box<T>>),
}

#[derive(Default)]
pub struct Container<T>
    where T: Default
{
    val: ManuallyDrop<Box<T>>,
    chan: Option<SyncSender<Message<T>>>,
}

impl<T> Deref for Container<T>
    where T: Default
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl<T> DerefMut for Container<T>
    where T: Default
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.val
    }
}

impl<T> Drop for Container<T>
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
            });
    }
}

struct Slot<T> {
    val: *mut ManuallyDrop<Box<T>>,
    next: AtomicPtr<Slot<T>>,
}

impl<T> Slot<T>
    where T: Default
{
    fn new(val: T) -> Slot<T> {
        let mut wrapper = ManuallyDrop::new(Box::new(val));
        Slot {
            val: &mut wrapper as *mut ManuallyDrop<Box<T>>,
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    fn take(mut self, chan: SyncSender<Message<T>>) -> Container<T> {
        Container {
            val: unsafe { self.val.read() },
            chan: Some(chan),
        }
    }
}

struct Inner<T> {
    head: *mut Slot<T>,
    tail: *mut Slot<T>,
    chan: (SyncSender<Message<T>>, Receiver<Message<T>>),
    close: AtomicBool,
}

impl<T> Inner<T>
    where T: Default
{
    fn get(&mut self) -> Container<T> {
        if self.close.load(Ordering::Acquire) {
            return Default::default()
        }

        if let Ok(msg) = self.chan.1.try_recv() {
            let val = match msg {
                Message::Shutdown => {
                    self.close.store(true, Ordering::SeqCst);
                    Default::default()
                }
                Message::Release(p) => {
                    Container {
                        val: unsafe { ptr::read(p) },
                        chan: Some(self.chan.0.clone()),
                    }
                }
            };

            return val;
        }

        Default::default()
    }
}

pub struct Pool<T> {
    inner: Arc<Inner<T>>,
}