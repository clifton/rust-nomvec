#![feature(allocator_api)]
use std::alloc::{Allocator, Global, Layout};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};

#[derive(Debug)]
pub enum AllocationError {
    CapacityOverflow,
    AllocationTooLarge,
    AllocationFailed,
}

impl std::fmt::Display for AllocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocationError::CapacityOverflow => write!(f, "Capacity overflow"),
            AllocationError::AllocationTooLarge => {
                write!(f, "Allocation too large")
            }
            AllocationError::AllocationFailed => write!(f, "Allocation failed"),
        }
    }
}

impl std::error::Error for AllocationError {}

struct RawVec<T, A: Allocator> {
    ptr: NonNull<T>,
    cap: usize,
    alloc: A,
    _marker: PhantomData<T>,
}

impl<T, A: Allocator> RawVec<T, A> {
    fn new(alloc: A) -> Self {
        let cap = if mem::size_of::<T>() == 0 {
            usize::MAX
        } else {
            0
        };
        // NonNull::dangling() doubles as "unallocated" and "zero-sized allocation"
        RawVec {
            ptr: NonNull::dangling(),
            cap,
            alloc,
            _marker: PhantomData,
        }
    }

    fn grow(&mut self) -> Result<(), AllocationError> {
        // since we set the capacity to usize::MAX when elem_size is
        // 0, getting to here necessarily means the Vec is overfull.
        if mem::size_of::<T>() == 0 {
            return Err(AllocationError::CapacityOverflow);
        }

        let new_cap = if self.cap == 0 {
            4 // Start with a small capacity
        } else {
            // Grow by ~1.5x, which is a good balance between memory usage and performance
            self.cap + (self.cap >> 1)
        };

        // Check for potential overflow
        let new_cap = new_cap.min(isize::MAX as usize);
        let new_layout = Layout::array::<T>(new_cap).unwrap();

        if new_layout.size() > isize::MAX as usize {
            return Err(AllocationError::AllocationTooLarge);
        }

        let new_ptr = if self.cap == 0 {
            self.alloc.allocate(new_layout)
        } else {
            let old_layout = Layout::array::<T>(self.cap).unwrap();
            unsafe {
                self.alloc
                    .grow(self.ptr.cast::<u8>(), old_layout, new_layout)
            }
        };
        // if allocation fails, `new_ptr` will be null in which case we will return an error
        self.ptr = match new_ptr {
            Ok(ptr) => ptr.cast(),
            Err(_) => return Err(AllocationError::AllocationFailed),
        };
        self.cap = new_cap;
        Ok(())
    }
}

impl<T, A: Allocator> Drop for RawVec<T, A> {
    fn drop(&mut self) {
        if self.cap != 0 {
            let elem_size = mem::size_of::<T>();

            // don't free zero-sized allocations, as they were never allocated.
            if self.cap != 0 && elem_size != 0 {
                let align = mem::align_of::<T>();
                let num_bytes = elem_size * self.cap;
                let layout = Layout::from_size_align(num_bytes, align).unwrap();
                unsafe {
                    self.alloc.deallocate(self.ptr.cast::<u8>(), layout);
                }
            }
        }
    }
}

pub struct NomVec<T, A: Allocator = Global> {
    buf: RawVec<T, A>,
    len: usize,
}

impl<T, A: Allocator + Default> Default for NomVec<T, A> {
    fn default() -> Self {
        Self::new(A::default())
    }
}

impl<T, A: Allocator> NomVec<T, A> {
    fn ptr(&self) -> *mut T {
        self.buf.ptr.as_ptr()
    }

    fn cap(&self) -> usize {
        self.buf.cap
    }

    pub fn new(alloc: A) -> Self {
        Self {
            buf: RawVec::new(alloc),
            len: 0,
        }
    }

    pub fn push(&mut self, elem: T) {
        if self.len == self.cap() {
            self.buf.grow().unwrap();
        }
        unsafe {
            ptr::write(self.ptr().add(self.len), elem);
        }
        // Can't fail, we'll OOM first.
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            unsafe { Some(ptr::read(self.ptr().add(self.len))) }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn insert(&mut self, index: usize, elem: T) {
        // Note: `<=` because it's valid to insert after everything
        // which would be equivalent to push.
        assert!(index <= self.len, "index out of bounds");
        if self.cap() == self.len {
            self.buf.grow().unwrap();
        }
        unsafe {
            if index < self.len {
                ptr::copy(
                    self.ptr().add(index),
                    self.ptr().add(index + 1),
                    self.len - index,
                );
            }
            ptr::write(self.ptr().add(index), elem);
            self.len += 1;
        }
    }

    pub fn remove(&mut self, index: usize) -> T {
        assert!(index < self.len, "index out of bounds");
        unsafe {
            self.len -= 1;
            let result = ptr::read(self.ptr().add(index));
            ptr::copy(
                self.ptr().add(index + 1),
                self.ptr().add(index),
                self.len - index,
            );
            result
        }
    }

    pub fn drain(&mut self) -> Drain<'_, T, A> {
        unsafe {
            let iter = RawValIter::new(self);
            // this is a mem::forget safety thing. If Drain is forgotten, we just
            // leak the whole Vec's contents. Also we need to do this *eventually*
            // anyway, so why not do it now?
            self.len = 0;
            Drain {
                iter,
                vec: PhantomData,
            }
        }
    }
}

impl<T, A: Allocator> Drop for NomVec<T, A> {
    fn drop(&mut self) {
        // deallocation is handled by RawVec
        while self.pop().is_some() {}
    }
}

impl<T, A: Allocator> Deref for NomVec<T, A> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { ::std::slice::from_raw_parts(self.ptr(), self.len) }
    }
}

impl<T, A: Allocator> DerefMut for NomVec<T, A> {
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { ::std::slice::from_raw_parts_mut(self.ptr(), self.len) }
    }
}

impl<T, A: Allocator> IntoIterator for NomVec<T, A> {
    type Item = T;
    type IntoIter = IntoIter<T, A>;

    fn into_iter(self) -> IntoIter<T, A> {
        unsafe {
            // need to use ptr::read to unsafely move the buf out since it's
            // not Copy, and Vec implements Drop (so we can't destructure it).
            let iter = RawValIter::new(&self);
            let buf = ptr::read(&self.buf);
            mem::forget(self);
            IntoIter { iter, _buf: buf }
        }
    }
}

struct RawValIter<T> {
    start: *const T,
    end: *const T,
}

impl<T> RawValIter<T> {
    // unsafe to construct because it has no associated lifetimes.
    // This is necessary to store a RawValIter in the same struct as
    // its actual allocation. OK since it's a private implementation
    // detail.
    unsafe fn new(slice: &[T]) -> Self {
        let start = slice.as_ptr();
        RawValIter {
            start,
            end: if mem::size_of::<T>() == 0 {
                ((start as usize) + slice.len()) as *const _
            } else if slice.is_empty() {
                // if `len = 0`, then this is not actually allocated memory.
                // Need to avoid offsetting because that will give wrong
                // information to LLVM via GEP.
                start
            } else {
                start.add(slice.len())
            },
        }
    }
}

impl<T> Iterator for RawValIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                let result = ptr::read(self.start);
                self.start = if mem::size_of::<T>() == 0 {
                    (self.start as usize + 1) as *const _
                } else {
                    self.start.offset(1)
                };
                Some(result)
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len =
            (self.end as usize - self.start as usize) / mem::size_of::<T>();
        (len, Some(len))
    }
}

impl<T> DoubleEndedIterator for RawValIter<T> {
    fn next_back(&mut self) -> Option<T> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                self.end = self.end.offset(-1);
                Some(ptr::read(self.end))
            }
        }
    }
}

pub struct IntoIter<T, A: Allocator> {
    _buf: RawVec<T, A>,
    iter: RawValIter<T>,
}

impl<T, A: Allocator> Iterator for IntoIter<T, A> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T, A: Allocator> DoubleEndedIterator for IntoIter<T, A> {
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back()
    }
}

impl<T, A: Allocator> Drop for IntoIter<T, A> {
    fn drop(&mut self) {
        // only need to ensure all our elements are read;
        // buffer will clean itself up afterwards.
        for _ in &mut self.iter {}
    }
}

pub struct Drain<'a, T: 'a, A: Allocator + 'a> {
    // Need to bound the lifetime here, so we do it with `&'a mut Vec<T>`
    // because that's semantically what we contain. We're "just" calling
    // `pop()` and `remove(0)`.
    vec: PhantomData<&'a mut NomVec<T, A>>,
    iter: RawValIter<T>,
}

impl<'a, T, A: Allocator> Iterator for Drain<'a, T, A> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.iter.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T, A: Allocator> DoubleEndedIterator for Drain<'a, T, A> {
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back()
    }
}

impl<'a, T, A: Allocator> Drop for Drain<'a, T, A> {
    fn drop(&mut self) {
        // pre-drain the iter
        for _ in &mut self.iter {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_push() {
        let mut cv = NomVec::new(Global);
        cv.push(2);
        assert_eq!(cv.len(), 1);
        cv.push(3);
        assert_eq!(cv.len(), 2);
    }

    #[test]
    fn vec_iter() {
        let mut cv = NomVec::new(Global);
        cv.push(2);
        cv.push(3);
        let mut accum = 0;
        for x in cv.iter() {
            accum += x;
        }
        assert_eq!(accum, 5);
    }

    #[test]
    fn vec_into_iter() {
        let mut cv = NomVec::new(Global);
        cv.push(2);
        cv.push(3);
        assert_eq!(cv.into_iter().collect::<Vec<i32>>(), vec![2, 3]);
    }

    #[test]
    fn vec_into_double_ended_iter() {
        let mut cv = NomVec::new(Global);
        cv.push(2);
        cv.push(3);
        assert_eq!(*cv.iter().next_back().unwrap(), 3);
    }

    #[test]
    fn vec_pop() {
        let mut cv = NomVec::new(Global);
        cv.push(2);
        assert_eq!(cv.len(), 1);
        cv.pop();
        assert_eq!(cv.len(), 0);
        assert!(cv.pop().is_none());
    }

    #[test]
    fn vec_insert() {
        let mut cv: NomVec<i32, Global> = NomVec::new(Global);
        cv.insert(0, 2); // test insert at end
        cv.insert(0, 1); // test insert at beginning
        assert_eq!(cv.pop().unwrap(), 2);
    }

    #[test]
    fn vec_remove() {
        let mut cv = NomVec::new(Global);
        cv.push(2);
        assert_eq!(cv.remove(0), 2);
        assert_eq!(cv.len(), 0);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn vec_cant_remove() {
        let mut cv: NomVec<i32, Global> = NomVec::new(Global);
        cv.remove(0);
    }

    #[test]
    fn vec_drain() {
        let mut cv = NomVec::new(Global);
        cv.push(1);
        cv.push(2);
        cv.push(3);
        assert_eq!(cv.len(), 3);
        {
            let mut drain = cv.drain();
            assert_eq!(drain.next().unwrap(), 1);
            assert_eq!(drain.next_back().unwrap(), 3);
        }
        assert_eq!(cv.len(), 0);
    }

    #[test]
    fn vec_zst() {
        let mut v = NomVec::new(Global);
        for _i in 0..10 {
            v.push(());
        }
        assert_eq!(v.len(), 10);

        let mut count = 0;
        for _ in v.into_iter() {
            count += 1
        }
        assert_eq!(10, count);
    }

    #[test]
    fn test_many_allocations() {
        let mut cv = NomVec::new(Global);
        for i in 0..10_000_000 {
            cv.push(i);
        }
        assert_eq!(cv.len(), 10_000_000);
        assert_eq!(cv[999_999], 999_999);
        assert!(cv.cap() > 10_000_000);
    }
}
