#![allow(unused)]

use std::sync::atomic;
use crate::pool::SLOT_CAP;

const GET_MASK: u16 = 0b0101_0101_0101_0101;
const PUT_MASK: u16 = 0b0000_0000_0000_0000;

#[inline(always)]
pub(crate) fn cpu_relax(count: usize) {
    for _ in 0..(1 << count) {
        atomic::spin_loop_hint()
    }
}

/// Assuming we have 8 elements per slot, otherwise must update the assumption.
pub(crate) fn locate(src: u16, get: bool) -> Result<usize, ()> {
    let mut pos: u32 = if get {
        (src & GET_MASK).trailing_zeros()
    } else {
        src.trailing_zeros()
    };

    Err(())
}

/// 2 * pos + 1 is where the access bit locates for slice position `pos`
#[inline(always)]
pub(crate) fn in_state(origin: u16, pos: u16) -> Result<u16, ()> {
    // if the bit is marked for free to edit, mark the bit
    if (origin & (0b1 << (2 * pos + 1))) == 0 {
        return Ok(origin | (0b1 << (2 * pos + 1)))
    }

    // already marked, skip
    Err(())
}

/// 2 * pos + 1 is where the access bit locates for slice position `pos`
#[inline(always)]
pub(crate) fn out_state(origin: u16, pos: u16) -> Result<u16, ()> {
    // only update if the position is marked, otherwise it will be deadlocked
    if (origin & (0b1 << (2 * pos + 1))) > 0 {
        return Ok(origin ^ (0b11 << (2 * pos)));
    }

    // the bit not marked for being edited? skip
    Err(())
}
