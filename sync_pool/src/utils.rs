#![allow(unused)]

use std::sync::atomic;
use crate::pool::SLOT_CAP;

const GET_MASK: u16 = 0b1010_1010_1010_1010;
const PUT_MASK: u16 = 0b1111_1111_1111_1111;

#[inline(always)]
pub(crate) fn cpu_relax(count: usize) {
    for _ in 0..(1 << count) {
        atomic::spin_loop_hint()
    }
}

/// Assuming we have 8 elements per slot, otherwise must update the assumption.
pub(crate) fn access(src: u16, get: bool) -> Result<u16, ()> {
    let mut pos = 0;
    let mut base = if get {
        src ^ GET_MASK
    } else {
        src ^ PUT_MASK
    };

    while base > 0 {
        if (base & 0b11) == 0b11 {
            return in_state(src, pos);
        }

        pos += 1;
        base >>= 2;
    }

    Err(())
}

/// 2 * pos + 1 is where the access bit locates for slice position `pos`
#[inline(always)]
pub(crate) fn in_state(origin: u16, pos: u16) -> Result<u16, ()> {
    // if the bit is marked for free to edit, mark the bit
    if (origin & (0b10 << (2 * pos))) == 0 {
        return Ok(origin | (0b10 << (2 * pos)))
    }

    // already marked, skip
    Err(())
}

/// 2 * pos + 1 is where the access bit locates for slice position `pos`
#[inline(always)]
pub(crate) fn out_state(origin: u16, pos: u16) -> Result<u16, ()> {
    // only update if the position is marked, otherwise it will be deadlocked
    if (origin & (0b10 << (2 * pos))) > 0 {
        return Ok(origin ^ (0b11 << (2 * pos)));
    }

    // the bit not marked for being edited? skip
    Err(())
}


#[cfg(test)]
mod utils_test {
    use super::*;

    #[test]
    fn access_pass() {
        let test1 = 0b0101010001010100;
        assert_eq!(access(test1, false), Ok(0b0101010001010110));
        assert_eq!(access(test1, true),  Ok(0b0101010001011100));

        let test2 = 0b0101010001010101;
        assert_eq!(access(test2, false), Ok(0b0101011001010101));
        assert_eq!(access(test2, true),  Ok(0b0101010001010111));

        let test3 = 0b0101010001010111;
        assert_eq!(access(test3, false), Ok(0b0101011001010111));
        assert_eq!(access(test3, true),  Ok(0b0101010001011111));

        let test4 = 0b0101010001011011;
        assert_eq!(access(test4, false), Ok(0b0101011001011011));
        assert_eq!(access(test4, true),  Ok(0b0101010001111011));
    }

    #[test]
    fn access_deny() {
        let test1 = 0b0010000000000000;
        assert_eq!(access(test1, false), Ok(0b0010000000000010));
        assert_eq!(access(test1, true),  Err(()));

        let test2 = 0b0111010101010111;
        assert_eq!(access(test2, false), Err(()));
        assert_eq!(access(test2, true),  Ok(0b0111010101011111));
    }
}