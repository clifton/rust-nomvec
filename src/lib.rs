#![feature(ptr_internals)] // std::ptr::Unique
#![feature(alloc_internals)] // std::alloc::rust_oom
// #![feature(heap_api)] // heap::allocate

use std::mem;
use std::ops::{Deref, DerefMut};
use std::alloc::{alloc, realloc, Layout, dealloc, rust_oom};

use std::ptr::{self, Unique};

struct RawVec<T> {
    ptr: Unique<T>,
    cap: usize,
}

impl<T> RawVec<T> {
    fn new() -> Self {
        assert!(mem::size_of::<T>() != 0, "TODO: implement ZST support");
        RawVec { ptr: Unique::empty(), cap: 0, }
    }

    // unchanged from Vec
    fn grow(&mut self) {
        unsafe {
            let align = mem::align_of::<T>();
            let elem_size = mem::size_of::<T>();

            let (new_cap, ptr) = if self.cap == 0 {
                let layout = Layout::from_size_align_unchecked(elem_size, align);
                let ptr = alloc(layout);
                (1, ptr)
            } else {
                let new_cap = self.cap * 2;
                let old_num_bytes = self.cap * elem_size;
                assert!(
                    old_num_bytes <= (::std::isize::MAX as usize) / 2,
                    "Capacity overflow!"
                );
                let num_new_bytes = old_num_bytes * 2;
                let layout = Layout::from_size_align_unchecked(old_num_bytes, align);
                let ptr = realloc(self.ptr.as_ptr() as *mut _, layout, num_new_bytes);
                (new_cap, ptr)
            };

            // If allocate or reallocate fail, we'll get `null` back
            if ptr.is_null() {
                rust_oom(Layout::from_size_align_unchecked(
                    new_cap * elem_size,
                    align,
                ));
            }

            self.ptr = Unique::new(ptr as *mut _).unwrap();
            self.cap = new_cap;
        }
    }
}

impl<T> Drop for RawVec<T> {
    fn drop(&mut self) {
        if self.cap != 0 {
            let align = mem::align_of::<T>();
            let elem_size = mem::size_of::<T>();
            let num_bytes = elem_size * self.cap;
            unsafe {
                let layout = Layout::from_size_align_unchecked(num_bytes, align);
                dealloc(self.ptr.as_ptr() as *mut _, layout);
            }
        }
    }
}

pub struct CVec<T> {
    buf: RawVec<T>,
    len: usize,
}

impl<T> CVec<T> {
    fn ptr(&self) -> *mut T {
        self.buf.ptr.as_ptr()
    }

    fn cap(&self) -> usize {
        self.buf.cap
    }

    pub fn new() -> Self {
        Self { buf: RawVec::new(), len: 0, }
    }

    pub fn push(&mut self, elem: T) {
        if self.len == self.cap() { self.buf.grow(); }
        unsafe {
            ptr::write(self.ptr().offset(self.len as isize), elem);
        }
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            unsafe {
                Some(ptr::read(self.ptr().offset(self.len as isize)))
            }
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn insert(&mut self, index: usize, elem: T) {
        // Note: `<=` because it's valid to insert after everything
        // which would be equivalent to push.
        assert!(index <= self.len, "index out of bounds");
        if self.cap() == self.len { self.buf.grow(); }
        unsafe {
            if index < self.len {
                ptr::copy(self.ptr().offset(index as isize),
                          self.ptr().offset(index as isize + 1),
                          self.len - index
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
            ptr::copy(self.ptr().offset(index as isize + 1),
                      self.ptr().offset(index as isize),
                      self.len - index);
            result
        }
    }
}

impl<T> Drop for CVec<T> {
    fn drop(&mut self) {
        // deallocation is handled by RawVec
        while let Some(_) = self.pop() {}
    }
}

impl<T> Deref for CVec<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe {
            ::std::slice::from_raw_parts(self.ptr(), self.len)
        }
    }
}

impl<T> DerefMut for CVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        unsafe {
            ::std::slice::from_raw_parts_mut(self.ptr(), self.len)
        }
    }
}

struct IntoIter<T> {
    _buf: RawVec<T>,
    start: *const T,
    end: *const T,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.start == self.end {
            None
        } else {
            unsafe {
                let result = ptr::read(self.start);
                self.start = self.start.offset(1);
                Some(result)
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.end as usize - self.start as usize)
                  / mem::size_of::<T>();
        (len, Some(len))
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
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

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        // only need to ensure all our elements are read;
        // buffer will clean itself up afterwards.
        for _ in &mut *self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_push() {
        let mut cv = CVec::new();
        cv.push(2);
        assert_eq!(cv.len(), 1);
        cv.push(3);
        assert_eq!(cv.len(), 2);
    }

    #[test]
    fn vec_iter() {
        let mut cv = CVec::new();
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
        let mut cv = CVec::new();
        cv.push(2);
        cv.push(3);
        assert_eq!(format!("{:?}", cv.into_iter()), "Iter([2, 3])");
    }

    #[test]
    fn vec_into_double_ended_iter() {
        let mut cv = CVec::new();
        cv.push(2);
        cv.push(3);
        assert_eq!(*cv.iter().next_back().unwrap(), 3);
    }

    #[test]
    fn vec_pop() {
        let mut cv = CVec::new();
        cv.push(2);
        assert_eq!(cv.len(), 1);
        cv.pop();
        assert_eq!(cv.len(), 0);
        assert!(cv.pop() == None);
    }

    #[test]
    fn vec_insert() {
        let mut cv: CVec<i32> = CVec::new();
        cv.insert(0, 2); // test insert at end
        cv.insert(0, 1); // test insert at beginning
        assert_eq!(cv.pop().unwrap(), 2);
    }

    #[test]
    fn vec_remove() {
        let mut cv = CVec::new();
        cv.push(2);
        assert_eq!(cv.remove(0), 2);
        assert_eq!(cv.len(), 0);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn vec_cant_remove() {
        let mut cv: CVec<i32> = CVec::new();
        cv.remove(0);
    }
}
