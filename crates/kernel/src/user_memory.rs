//! Audited access to raw buffers supplied by EL0 tasks.
//!
//! K2 has no MMU or address-space isolation, so the kernel cannot prove that
//! an arbitrary non-null EL0 address is mapped. This module still validates
//! integer conversions, null/length combinations, and range overflow. It is
//! deliberately the only place that dereferences retained user addresses.

use core::cmp;
use core::mem::size_of;
use core::ptr;

use trains_primitives::task::TaskId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidUserBuffer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadBuffer {
    address: usize,
    length: usize,
}

impl ReadBuffer {
    pub(crate) fn decode(address: u64, length: u64) -> Result<Self, InvalidUserBuffer> {
        let (address, length) = decode_byte_range(address, length)?;
        Ok(Self { address, length })
    }

    pub(crate) const fn length(self) -> usize {
        self.length
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WriteBuffer {
    address: usize,
    capacity: usize,
}

impl WriteBuffer {
    pub(crate) fn decode(address: u64, capacity: u64) -> Result<Self, InvalidUserBuffer> {
        let (address, capacity) = decode_byte_range(address, capacity)?;
        Ok(Self { address, capacity })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TaskIdDestination {
    address: usize,
}

impl TaskIdDestination {
    pub(crate) fn decode(address: u64) -> Result<Self, InvalidUserBuffer> {
        let address = usize::try_from(address).map_err(|_| InvalidUserBuffer)?;
        if address == 0 || address.checked_add(size_of::<u32>() - 1).is_none() {
            return Err(InvalidUserBuffer);
        }
        Ok(Self { address })
    }
}

fn decode_byte_range(address: u64, length: u64) -> Result<(usize, usize), InvalidUserBuffer> {
    let address = usize::try_from(address).map_err(|_| InvalidUserBuffer)?;
    let length = usize::try_from(length).map_err(|_| InvalidUserBuffer)?;

    // Rust allocations and slices cannot exceed `isize::MAX`, and reserving
    // the upper half of the result domain keeps syscall error values distinct.
    if length > isize::MAX as usize {
        return Err(InvalidUserBuffer);
    }
    if length == 0 {
        return Ok((address, length));
    }
    if address == 0 || address.checked_add(length - 1).is_none() {
        return Err(InvalidUserBuffer);
    }

    Ok((address, length))
}

/// Copies bytes between decoded user buffers and returns the copied length.
///
/// `ptr::copy` intentionally permits overlap. A zero-byte copy performs no
/// pointer operation, which makes a null address valid exactly for an empty
/// buffer.
pub(crate) fn copy(source: ReadBuffer, destination: WriteBuffer) -> usize {
    let length = cmp::min(source.length, destination.capacity);
    if length != 0 {
        // SAFETY: the descriptors passed structural validation above. With no
        // MMU, mapped-access validity is part of the trusted EL0 ABI. All raw
        // user-memory dereferences are kept in this audited module.
        unsafe {
            ptr::copy(
                source.address as *const u8,
                destination.address as *mut u8,
                length,
            );
        }
    }
    length
}

pub(crate) fn write_task_id(destination: TaskIdDestination, id: TaskId) {
    // SAFETY: `destination` is non-null and its complete u32 range did not
    // overflow. Unaligned writes make alignment a non-requirement of the raw
    // ABI; mapped-access validity remains part of the trusted EL0 contract.
    unsafe {
        ptr::write_unaligned(destination.address as *mut u32, id.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_is_valid_only_for_empty_byte_buffers() {
        assert!(ReadBuffer::decode(0, 0).is_ok());
        assert!(WriteBuffer::decode(0, 0).is_ok());
        assert_eq!(ReadBuffer::decode(0, 1), Err(InvalidUserBuffer));
        assert_eq!(WriteBuffer::decode(0, 1), Err(InvalidUserBuffer));
        assert_eq!(TaskIdDestination::decode(0), Err(InvalidUserBuffer));
    }

    #[test]
    fn copy_is_byte_oriented_and_truncates_without_a_terminator() {
        let source = *b"abcd";
        let mut destination = [0xa5; 3];
        let source = ReadBuffer::decode(source.as_ptr() as usize as u64, 4).unwrap();
        let destination_buffer =
            WriteBuffer::decode(destination.as_mut_ptr() as usize as u64, 2).unwrap();

        assert_eq!(copy(source, destination_buffer), 2);
        assert_eq!(destination, [b'a', b'b', 0xa5]);
    }
}
