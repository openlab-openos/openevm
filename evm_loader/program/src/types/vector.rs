use crate::allocator::{acc_allocator, StateAccountAllocator};
use allocator_api2::SliceExt;

pub type Vector<T> = allocator_api2::vec::Vec<T, StateAccountAllocator>;

#[macro_export]
macro_rules! vector {
    () => (
        allocator_api2::vec::Vec::new_in($crate::allocator::acc_allocator())
    );
    ($elem:expr; $n:expr) => (
        allocator_api2::vec::from_elem_in($elem, $n, $crate::allocator::acc_allocator())
    );
    ($($x:expr),+ $(,)?) => (
        allocator_api2::boxed::Box::<[_], $crate::allocator::StateAccountAllocator>::into_vec(
            allocator_api2::boxed::Box::slice(
                allocator_api2::boxed::Box::new_in([$($x),+], $crate::allocator::acc_allocator())
            )
        )
    );
}

pub trait VectorVecExt<T> {
    fn into_vector(self) -> Vector<T>
    where
        T: Copy + Default;
}

pub trait VectorSliceExt<T> {
    fn to_vector(&self) -> Vector<T>
    where
        T: Copy + Default;
}

pub trait VectorVecSlowExt<T> {
    fn elementwise_copy_into_vector(self) -> Vector<T>
    where
        T: Clone;
}

pub trait VectorSliceSlowExt<T> {
    fn elementwise_copy_to_vector(&self) -> Vector<T>
    where
        T: Clone;
}

impl<T: Copy> VectorVecExt<T> for Vec<T> {
    fn into_vector(self) -> Vector<T> {
        let mut ret = Vector::with_capacity_in(self.len(), crate::allocator::acc_allocator());
        // SAFETY:
        // allocated above with the capacity of `self.len()`, and initialize to `self.len()` in
        // ptr::copy_to_non_overlapping below.
        unsafe {
            self.as_ptr()
                .copy_to_nonoverlapping(ret.as_mut_ptr(), self.len());
            ret.set_len(self.len());
        }
        ret
    }
}

impl<T: Copy> VectorSliceExt<T> for [T] {
    fn to_vector(&self) -> Vector<T> {
        let mut ret = Vector::with_capacity_in(self.len(), crate::allocator::acc_allocator());
        // SAFETY:
        // allocated above with the capacity of `self.len()`, and initialize to `self.len()` in
        // ptr::copy_to_non_overlapping below.
        unsafe {
            self.as_ptr()
                .copy_to_nonoverlapping(ret.as_mut_ptr(), self.len());
            ret.set_len(self.len());
        }
        ret
    }
}

impl<T> VectorSliceSlowExt<T> for [T] {
    fn elementwise_copy_to_vector(&self) -> Vector<T>
    where
        T: Clone,
    {
        SliceExt::to_vec_in(self, acc_allocator())
    }
}

impl<T> VectorVecSlowExt<T> for Vec<T> {
    fn elementwise_copy_into_vector(self) -> Vector<T> {
        let mut ret = Vector::with_capacity_in(self.len(), acc_allocator());
        for item in self {
            ret.push(item);
        }
        ret
    }
}
