#![allow(unused)]

use crate::bucket::SLOT_CAP;
use std::sync::atomic;

const GET_MASK: u16 = 0b1010_1010_1010_1010;
const PUT_MASK: u16 = 0b1111_1111_1111_1111;

#[inline(always)]
pub(crate) fn cpu_relax(count: usize) {
    for _ in 0..(1 << count) {
        atomic::spin_loop_hint()
    }
}

/// Assuming we have 8 elements per slot, otherwise must update the assumption.
pub(crate) fn enter(src: u16, get: bool) -> Result<(u16, u16), ()> {
    let mut pos = 0;
    let mut base = if get { src ^ GET_MASK } else { src ^ PUT_MASK };

    while base > 0 {
        if (base & 0b11) == 0b11 {
            // update the state and the position
            if let Ok(state) = in_state(src, 2 * pos) {
                return Ok((state, pos));
            }

            // shouldn't make it here
            return Err(());
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

/// `2 * pos` -> `padded_pos` is where the access bit locates for slice position `pos`
#[inline(always)]
fn in_state(origin: u16, pad_pos: u16) -> Result<u16, ()> {
    // the intended state after mark the access bit
    let next = origin | (0b10 << pad_pos);

    // if the marked state is the same as the origin, meaning the src pos is already accessed, quit
    // with error
    if next == origin {
        return Err(());
    }

    // done
    Ok(next)
}

/// `2 * pos` -> `padded_pos` is where the access bit locates for slice position `pos`
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
        assert_eq!(access(test1, false), Ok(0b0101010001010110));
        assert_eq!(access(test1, true), Ok(0b0101010001011100));

        let test2 = 0b0101010001010101;
        assert_eq!(access(test2, false), Ok(0b0101011001010101));
        assert_eq!(access(test2, true), Ok(0b0101010001010111));

        let test3 = 0b0101010001010111;
        assert_eq!(access(test3, false), Ok(0b0101011001010111));
        assert_eq!(access(test3, true), Ok(0b0101010001011111));

        let test4 = 0b0101010001011011;
        assert_eq!(access(test4, false), Ok(0b0101011001011011));
        assert_eq!(access(test4, true), Ok(0b0101010001111011));
    }

    #[test]
    fn access_deny() {
        let test1 = 0b0010000000000000;
        assert_eq!(access(test1, false), Ok(0b0010000000000010));
        assert_eq!(access(test1, true), Err(()));

        let test2 = 0b0111010101010111;
        assert_eq!(access(test2, false), Err(()));
        assert_eq!(access(test2, true), Ok(0b0111010101011111));
    }
}
