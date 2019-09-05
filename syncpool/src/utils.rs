#![allow(unused)]

use crate::bucket::SLOT_CAP;
use std::sync::atomic;

const GET_MASK: u16 = 0b1010_1010_1010_1010;
const PUT_MASK: u16 = 0b1111_1111_1111_1111;
const FULL_FLAG: u16 = 0b0101_0101_0101_0101;

#[inline(always)]
pub(crate) fn cpu_relax(count: usize) {
    for _ in 0..(1 << count) {
        atomic::spin_loop_hint()
    }
}

pub(crate) fn check_len(src: u16) -> usize {
    match src & FULL_FLAG {
        0 => 0,
        FULL_FLAG => 8,
        mut base => {
            let mut count = 0;

            while base > 0 {
                if base & 1 == 1 {
                    count += 1;
                }

                base >>= 2;
            }

            count
        }
    }
}

/// Assuming we have 8 elements per slot, otherwise must update the assumption.
pub(crate) fn enter(src: u16, get: bool) -> Result<u16, ()> {
    // get the base bits to check on. If we're not going to meet the needs, terminate early.
    let mut base = if get {
        if src == 0 {
            return Err(());
        }

        src ^ GET_MASK
    } else {
        if src == FULL_FLAG {
            return Err(());
        }

        src ^ PUT_MASK
    };

    // find the starting position for the spot check
    let mut pos: u16 = {
        // a little trick: pre-calculate the starting point for finding the location
        let val = (base & PUT_MASK).trailing_zeros() as u16;

        // if bit 15 (or above) is 0, then we won't find a location in this bucket, skip the
        // remainder logic/loop.
        if val > 14 {
            return Err(());
        }

        if val % 2 == 1 {
            base >>= val + 1;
            (val + 1) / 2
        } else {
            base >>= val;
            val / 2
        }
    };

    while base > 0 {
        if base & 0b11 == 0b11 {
            // update the state and the position
            return Ok(pos);
        }

        pos += 1;
        base >>= 2;
    }

    Err(())
}

/// Assuming we have 8 elements per slot. A wrapper over the out-state
#[inline]
pub(crate) fn exit(src: u16, pos: u16) -> Result<u16, ()> {
    out_state(src, 2 * pos)
}

/// `2 * pos` -> `padded_pos` is where the enter bit locates for slice position `pos`
#[inline(always)]
fn in_state(origin: u16, pad_pos: u16) -> Result<u16, ()> {
    // the intended state after mark the enter bit
    let next = origin | (0b10 << pad_pos);

    // if the marked state is the same as the origin, meaning the src pos is already accessed, quit
    // with error
    if next == origin {
        return Err(());
    }

    // done
    Ok(next)
}

/// `2 * pos` -> `padded_pos` is where the enter bit locates for slice position `pos`
#[inline(always)]
fn out_state(origin: u16, pad_pos: u16) -> Result<u16, ()> {
    // only update if the position is marked, otherwise it will be deadlocked
    if (origin & (0b10 << pad_pos)) == 0 {
        return Err(());
    }

    // the bit not marked for being edited? skip
    Ok(origin ^ (0b11 << pad_pos))
}

#[cfg(test)]
mod utils_test {
    use super::*;

    #[test]
    fn access_pass() {
        let test1 = 0b0101010001010100;
        assert_eq!(enter(test1, false), Ok(0));
        assert_eq!(enter(test1, true), Ok(1));

        let test2 = 0b0101010001010101;
        assert_eq!(enter(test2, false), Ok(4));
        assert_eq!(enter(test2, true), Ok(0));

        let test3 = 0b0101010001010111;
        assert_eq!(enter(test3, false), Ok(4));
        assert_eq!(enter(test3, true), Ok(1));

        let test4 = 0b0101010001011011;
        assert_eq!(enter(test4, false), Ok(4));
        assert_eq!(enter(test4, true), Ok(2));
    }

    #[test]
    fn access_deny() {
        let test1 = 0b0010000000000000;
        assert_eq!(enter(test1, false), Ok(0));
        assert_eq!(enter(test1, true), Err(()));

        let test2 = 0b0111010101010111;
        assert_eq!(enter(test2, false), Err(()));
        assert_eq!(enter(test2, true), Ok(1));
    }
}
