#![feature(ptr_internals)] // std::ptr::Unique
#![feature(alloc_internals)] // std::alloc::*

use std::alloc::{self, dealloc, realloc, rust_oom, Layout};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};

struct RawVec<T> {
    ptr: NonNull<T>,
    cap: usize,
    _marker: PhantomData<T>,
}

impl<T> RawVec<T> {
    fn new() -> Self {
        let cap = if mem::size_of::<T>() == 0 {
            ::std::usize::MAX
        } else {
            0
        };
        // NonNull::dangling() doubles as "unallocated" and "zero-sized allocation"
        RawVec {
            ptr: NonNull::dangling(),
            cap,
            _marker: PhantomData,
        }
    }

    // unchanged from Vec
    fn grow(&mut self) {
        let elem_size = mem::size_of::<T>();
        let align = mem::align_of::<T>();
        // since we set the capacity to usize::MAX when elem_size is
        // 0, getting to here necessarily means the Vec is overfull.
        assert!(elem_size != 0, "capacity overflow");
        let (new_cap, ptr) = if self.cap == 0 {
            let new_layout = Layout::from_size_align(elem_size, align).unwrap();
            let ptr = unsafe { alloc::alloc(new_layout) };
            (1, ptr)
        } else {
            let new_cap = self.cap * 2;
            let old_num_bytes = self.cap * elem_size;
            assert!(
                old_num_bytes <= (::std::isize::MAX as usize) / 2,
                "Capacity overflow!"
            );
            let num_new_bytes = old_num_bytes * 2;
            let old_layout =
                Layout::from_size_align(old_num_bytes, align).unwrap();
            let ptr = unsafe {
                realloc(self.ptr.as_ptr() as *mut _, old_layout, num_new_bytes)
            };
            (new_cap, ptr)
        };

        // If allocate or reallocate fail, we'll get `null` back
        let new_layout =
            Layout::from_size_align(new_cap * elem_size, align).unwrap();
        if ptr.is_null() {
            rust_oom(new_layout);
        }
        self.ptr = match NonNull::new(ptr as *mut _) {
            Some(p) => p,
            None => alloc::handle_alloc_error(new_layout),
        };
        self.cap = new_cap;
    }
}

impl<T> Drop for RawVec<T> {
    fn drop(&mut self) {
        if self.cap != 0 {
            let elem_size = mem::size_of::<T>();

            // don't free zero-sized allocations, as they were never allocated.
            if self.cap != 0 && elem_size != 0 {
                let align = mem::align_of::<T>();
                let num_bytes = elem_size * self.cap;
                let layout = Layout::from_size_align(num_bytes, align).unwrap();
                unsafe {
                    dealloc(self.ptr.as_ptr() as *mut _, layout);
                }
            }
        }
    }
}

pub struct NomVec<T> {
    buf: RawVec<T>,
    len: usize,
}

impl<T> NomVec<T> {
    fn ptr(&self) -> *mut T {
        self.buf.ptr.as_ptr()
    }

    fn cap(&self) -> usize {
        self.buf.cap
    }

    pub fn new() -> Self {
        Self {
            buf: RawVec::new(),
            len: 0,
        }
    }

    pub fn push(&mut self, elem: T) {
        if self.len == self.cap() {
            self.buf.grow();
        }
        unsafe {
            ptr::write(self.ptr().offset(self.len as isize), elem);
        }
        // Can't fail, we'll OOM first.
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            unsafe { Some(ptr::read(self.ptr().offset(self.len as isize))) }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn insert(&mut self, index: usize, elem: T) {
        // Note: `<=` because it's valid to insert after everything
        // which would be equivalent to push.
        assert!(index <= self.len, "index out of bounds");
        if self.cap() == self.len {
            self.buf.grow();
        }
        unsafe {
            if index < self.len {
                ptr::copy(
                    self.ptr().offset(index as isize),
                    self.ptr().offset(index as isize + 1),
                    self.len - index,
                );
            }
            ptr::write(self.ptr().offset(index as isize), elem);
            self.len += 1;
        }
    }

    pub fn remove(&mut self, index: usize) -> T {
        assert!(index < self.len, "index out of bounds");
        unsafe {
            self.len -= 1;
            let result = ptr::read(self.ptr().offset(index as isize));
            ptr::copy(
                self.ptr().offset(index as isize + 1),
                self.ptr().offset(index as isize),
                self.len - index,
            );
            result
        }
    }

    pub fn drain(&mut self) -> Drain<T> {
        unsafe {
            let iter = RawValIter::new(&self);
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

impl<T> Drop for NomVec<T> {
    fn drop(&mut self) {
        // deallocation is handled by RawVec
        while let Some(_) = self.pop() {}
    }
}

impl<T> Deref for NomVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { ::std::slice::from_raw_parts(self.ptr(), self.len) }
    }
}

impl<T> DerefMut for NomVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe { ::std::slice::from_raw_parts_mut(self.ptr(), self.len) }
    }
}

impl<T> IntoIterator for NomVec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
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
        RawValIter {
            start: slice.as_ptr(),
            end: if mem::size_of::<T>() == 0 {
                ((slice.as_ptr() as usize) + slice.len()) as *const _
            } else if slice.len() == 0 {
                // if `len = 0`, then this is not actually allocated memory.
                // Need to avoid offsetting because that will give wrong
                // information to LLVM via GEP.
                slice.as_ptr()
            } else {
                slice.as_ptr().offset(slice.len() as isize)
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

pub struct IntoIter<T> {
    _buf: RawVec<T>,
    iter: RawValIter<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back()
    }
}

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        // only need to ensure all our elements are read;
        // buffer will clean itself up afterwards.
        for _ in &mut self.iter {}
    }
}

pub struct Drain<'a, T: 'a> {
    // Need to bound the lifetime here, so we do it with `&'a mut Vec<T>`
    // because that's semantically what we contain. We're "just" calling
    // `pop()` and `remove(0)`.
    vec: PhantomData<&'a mut NomVec<T>>,
    iter: RawValIter<T>,
}

impl<'a, T> Iterator for Drain<'a, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.iter.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T> DoubleEndedIterator for Drain<'a, T> {
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back()
    }
}

impl<'a, T> Drop for Drain<'a, T> {
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
        let mut cv = NomVec::new();
        cv.push(2);
        assert_eq!(cv.len(), 1);
        cv.push(3);
        assert_eq!(cv.len(), 2);
    }

    #[test]
    fn vec_iter() {
        let mut cv = NomVec::new();
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
        let mut cv = NomVec::new();
        cv.push(2);
        cv.push(3);
        assert_eq!(cv.into_iter().collect::<Vec<i32>>(), vec![2, 3]);
    }

    #[test]
    fn vec_into_double_ended_iter() {
        let mut cv = NomVec::new();
        cv.push(2);
        cv.push(3);
        assert_eq!(*cv.iter().next_back().unwrap(), 3);
    }

    #[test]
    fn vec_pop() {
        let mut cv = NomVec::new();
        cv.push(2);
        assert_eq!(cv.len(), 1);
        cv.pop();
        assert_eq!(cv.len(), 0);
        assert!(cv.pop() == None);
    }

    #[test]
    fn vec_insert() {
        let mut cv: NomVec<i32> = NomVec::new();
        cv.insert(0, 2); // test insert at end
        cv.insert(0, 1); // test insert at beginning
        assert_eq!(cv.pop().unwrap(), 2);
    }

    #[test]
    fn vec_remove() {
        let mut cv = NomVec::new();
        cv.push(2);
        assert_eq!(cv.remove(0), 2);
        assert_eq!(cv.len(), 0);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn vec_cant_remove() {
        let mut cv: NomVec<i32> = NomVec::new();
        cv.remove(0);
    }

    #[test]
    fn vec_drain() {
        let mut cv = NomVec::new();
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
        let mut v = NomVec::new();
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
}
