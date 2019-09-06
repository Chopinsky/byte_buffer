//! This module contains APIs that allocate large struct directly on the heap, such that we can
//! overcome the obstacle of using the `Box::new` syntax in certain situations, for example, a struct
//! that is too large to fit into the default thread stack. The syntax limit is caused by how `box`
//! is currently created: the struct will be created on the stack, and then moved to the heap, which
//! implies that the struct must be first of all be able to fit into the (limited) stack size in the
//! first place.
//!
//! This limit has been quite inconvenient for buffer struct where a buffer array can be of `MB` in
//! size. Using APIs provided by this module can help mitigate the gap -- we will allocate a well aligned
//! memory in the heap, where caller can pack the memory with valid and meaningful values.
//!
//! That said, the APIs can be extremely dangerous for struct that can be undefined if not properly
//! initialized. There are 2 APIs marked as `safe`, which provides ways to initialize the object before
//! yielding the instance to the caller, which could provide some warrants that the crafted struct
//! shall be valid and away from undefined behaviors.
//!
//! # Examples
//!
//! ```rust
//! use syncpool;
//!
//! struct BigStruct {
//!     a: u32,
//!     b: u32,
//!     c: [u8; 0x1_000_000],
//!     d: Vec<u8>,
//! }
//!
//! // create the object on the heap directly
//! let big: Box<BigStruct> = syncpool::make_box(|mut src: Box<BigStruct>| {
//!     src.a = 1;
//!     src.b = 42;
//!
//!     for i in 0..0x1_000_000 {
//!         src.c[i] = (i % 256) as u8;
//!     }
//!
//!     src.d = Vec::with_capacity(0x1_000_000);
//!     for i in 0..0x1_000_000 {
//!         src.d.push((i % 256) as u8)
//!     }
//!
//!     src
//! });
//!
//! assert_eq!(big.a, 1);
//! assert_eq!(big.b, 42);
//!
//! assert_eq!(big.c[255], 255);
//! assert_eq!(big.c[4200], 104);
//!
//! assert_eq!(big.d[255], 255);
//! assert_eq!(big.d[4200], 104);
//! ```
#![allow(unused)]

use std::alloc::{alloc, alloc_zeroed, Layout};
use std::ptr;

/// Create a box structure without moving the wrapped value from the stack to the heap. This API is
/// most useful when the wrapped value is too large for the default stack size, such that initializing
/// and packing the valuing into the box is a pain.
///
/// Note that calling the API is unsafe, because it only creates a well-aligned memory structure in
/// the heap, but all fields are in the state of undefined behavior at the moment. You *must* initialize
/// the fields with default values, or pack it with meaningful placeholders. Using the object directly
/// after being created by the API is *extremely* dangerous and will almost certainly lead to undefined
/// behaviors.
///
/// # Examples
///
/// Create a boxed `BigStruct`
///
/// ```
/// use syncpool::raw_box;
///
/// struct BigStruct {
///     a: u32,
///     b: u32,
///     c: [u8; 0x1_000_000],
/// }
///
/// // create the object on the heap directly
/// let mut big: Box<BigStruct> = unsafe { raw_box::<BigStruct>() };
///
/// // initialize the fields
/// big.a = 0;
/// big.b = 42;
/// big.c = [0u8; 0x1_000_000];
///
/// // the fields are now valid
/// assert_eq!(big.c.len(), 0x1_000_000);
/// assert_eq!(big.c[4200], 0);
/// assert_eq!(big.a, 0);
/// ```
pub unsafe fn raw_box<T>() -> Box<T> {
    let layout = Layout::new::<T>();
    Box::from_raw(alloc(layout) as *mut T)
}

/// Similar to `raw_box`, this API creates a box structure without moving the wrapped value from the
/// stack to the heap. This API is most useful when the wrapped value is too large for the default
/// stack size, such that initializing and packing the valuing into the box is a pain.
///
/// The only difference is that all fields in the object will be initialized to 0. This will initialize
/// most primitive types, however, there is no warrant that the boxed object is valid or meaningful.
/// For example, if the source struct contains pointers or another `Box`ed object, the fields are still
/// undefined since they're pointing to the `null` pointer (i.e. default pointer created by
/// `std::ptr::null_mut()`).
///
/// # Examples
///
/// Create a boxed `DangerousStruct`
///
/// ```
/// use syncpool::raw_box_zeroed;
///
/// struct BigStruct {
///     a: u32,
///     b: u32,
///     c: [u8; 0x1_000_000],
/// }
///
/// // create the object on the heap directly
/// let mut big: Box<BigStruct> = unsafe { raw_box_zeroed::<BigStruct>() };
///
/// // the fields are now valid
/// assert_eq!(big.c.len(), 0x1_000_000);
/// assert_eq!(big.c[4200], 0);
/// assert_eq!(big.a, 0);
/// ```
pub unsafe fn raw_box_zeroed<T>() -> Box<T> {
    let layout = Layout::new::<T>();
    Box::from_raw(alloc_zeroed(layout) as *mut T)
}

/// This API is a wrapper on the unsafer version of the direct-to-the-heap-box APIs. The API is safe
/// because it is the caller's responsiblity to supply the struct initialier as a closure, such that
/// after calling the struct initializer, the returned object shall be valid and meaningful.
///
/// The closure will take the raw box object as the input parameter, which maybe invalid, and it is
/// the closure's responsiblity to assign valid values to the fields.
///
/// # Examples
///
/// Create the dangerous struct and pack valid values with it.
///
/// ```
/// use syncpool::{raw_box_zeroed, make_box};
/// use std::mem::MaybeUninit;
/// use std::ptr::NonNull;
/// use std::sync::atomic::{AtomicBool, Ordering};
///
/// struct BigStruct {
///     a: u32,
///     b: u32,
///     c: [u8; 0x1_000_000],
/// }
///
/// struct DangerousStruct {
///     a: u32,
///     b: MaybeUninit<AtomicBool>,
///     c: NonNull<BigStruct>,
/// }
///
/// // create the object directly on the heap
/// let mut boxed: Box<DangerousStruct> = make_box(|mut src: Box<DangerousStruct>| {
///     // initialize the fields in the handler
///     let mut big: &mut BigStruct = unsafe { Box::leak(raw_box_zeroed::<BigStruct>()) };
///     big.a = 42;
///     big.b = 4 * 42;
///     big.c[4200] = 125;
///
///     // make sure we initialize the fields
///     src.a = 42;
///     src.b = MaybeUninit::new(AtomicBool::new(false));
///     src.c = NonNull::new(big).unwrap();
///
///     src
/// });
///
/// // the fields are now valid
/// let big_ref = unsafe { boxed.c.as_ref() };
///
/// assert_eq!(big_ref.c.len(), 0x1_000_000);
/// assert_eq!(big_ref.c[4200], 125);
/// assert_eq!(big_ref.a, 42);
/// assert_eq!(big_ref.b, 168);
/// ```
pub fn make_box<T, F: Fn(Box<T>) -> Box<T>>(packer: F) -> Box<T> {
    let boxed = unsafe { raw_box_zeroed::<T>() };
    packer(boxed)
}

/// Similar to the `make_box` API, the `default_box` API is a wrapper over the unsafer version of the
/// directly-to-the-heap ones. If the struct wrapped in the box has implemented the `Default` trait,
/// then one can call this API that will invoke the `Default::default` to initialize the object, such
/// that the caller won't need to supply a closure to initialize all the fields.
///
/// # Examples
///
/// Create the box on the heap with the default implementation of the struct
///
/// ```rust
/// use syncpool::default_box;
/// use std::vec;
///
/// struct BigStruct {
///     a: u32,
///     b: u32,
///     c: Vec<u8>,
/// }
///
/// impl Default for BigStruct {
///     fn default() -> Self {
///         BigStruct {
///             a: 1,
///             b: 42,
///             c: vec::from_elem(0, 0x1_000_000),
///         }
///     }
/// }
///
/// // create the object directly on the heap
/// let boxed: Box<BigStruct> = default_box();
///
/// // the fields are now valid
/// assert_eq!(boxed.c.len(), 0x1_000_000);
/// assert_eq!(boxed.a, 1);
/// assert_eq!(boxed.b, 42);
///```
pub fn default_box<T: Default>() -> Box<T> {
    unsafe {
        let p = alloc(Layout::new::<T>()) as *mut T;
        ptr::write(p, Default::default());
        Box::from_raw(p)
    }
}

#[cfg(test)]
mod boxed_tests {
    use super::*;
    use std::{
        mem::MaybeUninit,
        ptr::NonNull,
        sync::atomic::{AtomicBool, Ordering},
        vec,
    };

    struct BigStruct {
        a: u32,
        b: u32,
        c: [u8; 0x1_000_000],
    }

    struct BigStruct2 {
        a: u32,
        b: u32,
        c: Vec<u8>,
    }

    impl Default for BigStruct2 {
        fn default() -> Self {
            BigStruct2 {
                a: 1,
                b: 42,
                c: vec::from_elem(0, 0x1_000_000),
            }
        }
    }

    struct DangerousStruct {
        a: u32,
        b: MaybeUninit<AtomicBool>,
        c: NonNull<BigStruct>,
    }

    impl Drop for DangerousStruct {
        fn drop(&mut self) {
            let _ = unsafe { Box::from_raw(self.c.as_ptr()) };
        }
    }

    fn make_test_box<T>(zeroed: bool) -> Box<T> {
        unsafe {
            if zeroed {
                raw_box_zeroed::<T>()
            } else {
                raw_box::<T>()
            }
        }
    }

    fn make_dangerous() -> Box<DangerousStruct> {
        let mut boxed = make_test_box::<DangerousStruct>(true);
        let mut big: &mut BigStruct = Box::leak(make_test_box::<BigStruct>(true));
        big.a = 42;
        big.b = 4 * 42;
        big.c[4200] = 125;

        // make sure we initialize the fields
        boxed.b = MaybeUninit::new(AtomicBool::new(false));
        boxed.c = NonNull::new(big).unwrap();

        boxed
    }

    #[test]
    fn raw_box_test() {
        let boxed = make_test_box::<BigStruct>(true);

        assert_eq!(boxed.c.len(), 0x1_000_000);
        assert_eq!(boxed.c[4200], 0);
        assert_eq!(boxed.a, 0);
    }

    #[test]
    fn pack() {
        // create the object on the heap directly
        let mut boxed: Box<DangerousStruct> = make_box(|mut src: Box<DangerousStruct>| {
            // initialize the fields in the handler
            let mut big: &mut BigStruct = unsafe { Box::leak(raw_box_zeroed::<BigStruct>()) };
            big.a = 42;
            big.b = 4 * 42;
            big.c[4200] = 125;

            // make sure we initialize the fields
            src.b = MaybeUninit::new(AtomicBool::new(false));
            src.c = NonNull::new(big).unwrap();

            src
        });

        // the fields are now valid
        let big_ref = unsafe { boxed.c.as_ref() };

        assert_eq!(big_ref.c.len(), 0x1_000_000);
        assert_eq!(big_ref.c[4200], 125);
        assert_eq!(big_ref.a, 42);
        assert_eq!(big_ref.b, 168);
    }

    #[test]
    fn init() {
        let mut boxed = make_test_box::<BigStruct>(false);
        boxed.a = 1;
        boxed.b = 42;
        boxed.c = [0u8; 0x1_000_000];

        assert_eq!(boxed.a, 1);
        assert_eq!(boxed.b, 42);
        assert_eq!(boxed.c.len(), 0x1_000_000);
        assert_eq!(boxed.c[4200], 0);
    }

    #[test]
    fn raw() {
        let mut boxed = make_dangerous();
        let big_ref = unsafe { boxed.c.as_ref() };

        assert_eq!(big_ref.a, 42);
        assert_eq!(big_ref.b, 168);
        assert_eq!(big_ref.c.len(), 0x1_000_000);
        assert_eq!(big_ref.c[4200], 125);

        let atomic = unsafe { &*boxed.b.as_ptr() };
        assert_eq!(atomic.load(Ordering::Acquire), false);
    }

    #[test]
    fn defaulted() {
        // create the object directly on the heap
        let mut boxed: Box<BigStruct2> = default_box();

        // the fields are now valid
        assert_eq!(boxed.c.len(), 0x1_000_000);
        assert_eq!(boxed.a, 1);
        assert_eq!(boxed.b, 42);
    }
}
