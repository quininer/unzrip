#![no_std]
#![cfg_attr(unzrip_i_am_nightly_and_i_want_fast, feature(core_intrinsics))]

use core::cell::UnsafeCell;


pub type Buf<'a> = &'a [ReadOnlyCell<u8>];

#[repr(transparent)]
pub struct ReadOnlyCell<T>(UnsafeCell<T>);

impl<T: Copy> ReadOnlyCell<T> {
    pub fn get(&self) -> T {
        unsafe {
            self.0.get().read_volatile()
        }
    }
}

pub mod slice {
    use core::slice;
    use crate::ReadOnlyCell;

    pub const unsafe fn from_ptr<'a, T>(ptr: *const T, len: usize) -> &'a [ReadOnlyCell<T>] {
        slice::from_raw_parts(ptr.cast(), len)
    }

    pub const fn from_slice<T>(input: &[T]) -> &[ReadOnlyCell<T>] {
        unsafe {
            from_ptr(input.as_ptr(), input.len())
        }
    }

    pub fn copy_from_slice<T: Copy>(dst: &mut [T], src: &[ReadOnlyCell<T>]) {
        assert_eq!(src.len(), dst.len());

        for (src, dst) in src.iter().zip(dst) {
            *dst = src.get();
        }
    }

    #[cfg(unzrip_i_am_nightly_and_i_want_fast)]
    pub fn copy_from_slice_nightly<T: Copy>(dst: &mut [T], src: &[ReadOnlyCell<T>]) {
        assert_eq!(src.len(), dst.len());

        unsafe {
            core::intrinsics::volatile_copy_nonoverlapping_memory(
                dst.as_mut_ptr(),
                src.as_ptr().cast(),
                src.len()
            );
        }
    }
}

impl<T: Copy + PartialEq> PartialEq<Self> for ReadOnlyCell<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl<T: Copy + PartialEq> PartialEq<T> for ReadOnlyCell<T> {
    fn eq(&self, other: &T) -> bool {
        self.get().eq(other)
    }
}

impl<T: Copy + PartialEq> Eq for ReadOnlyCell<T> {}

unsafe impl<T: Sync> Sync for ReadOnlyCell<T> {}
