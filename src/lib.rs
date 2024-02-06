use core::fmt::*;
use std::mem::{self, needs_drop, transmute};
use std::ops::{Index, IndexMut, Range};
use std::sync::atomic::Ordering;

use pi_arr::*;
use pi_null::Null;
use pi_share::ShareUsize;

extern crate pi_arr;


pub const DEFALLT_CAPACITY: usize = 32;

pub struct AppendVec<T: Null> {
    vec: Vec<T>,
    arr: Arr<T>,
    len: ShareUsize,
}
impl<T: Null> AppendVec<T> {
    /// Creates an empty [`AppendVec`] with the given capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut vec = AppendVec::with_capacity(3);
    /// let welcome: MessageKey = vec.insert("Welcome");
    /// let good_day = messages.insert("Good day");
    /// let hello = messages.insert("Hello");
    /// ```
    #[inline(always)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_multiple(capacity, 1)
    }
    /// Creates an empty [`AppendVec`] with the given capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut vec = AppendVec::with_capacity(3);
    /// let welcome: MessageKey = vec.insert("Welcome");
    /// let good_day = messages.insert("Good day");
    /// let hello = messages.insert("Hello");
    /// ```
    #[inline(always)]
    pub fn with_capacity_multiple(capacity: usize, multiple: usize) -> Self {
        let mut vec = Vec::with_capacity(capacity * multiple);
        vec.resize_with(vec.capacity(), || T::null());
        Self {
            vec,
            arr: Arr::new(),
            len: ShareUsize::new(0),
        }
    }
    /// 长度
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len() {
            return None;
        }
        if index < self.vec.capacity() {
            return Some(unsafe { self.vec.get_unchecked(index) });
        }
        self.arr.get(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        if index < self.vec.capacity() {
            return unsafe { self.vec.get_unchecked(index) };
        }
        self.arr.get_unchecked(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub unsafe fn get_0_unchecked(&self, index: usize) -> &T {
        return unsafe { self.vec.get_unchecked(index) };
    }
    #[inline(always)]
    pub unsafe fn get_1_unchecked(&self, index: usize) -> &T {
        self.arr.get_unchecked(self.vec.capacity() - index)
    }

    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let len = *self.len.get_mut();
        if index >= len {
            return None;
        }
        if index < self.vec.capacity() {
            return self.vec.get_mut(index);
        }
        self.arr.get_mut(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        if index < self.vec.capacity() {
            return self.vec.get_unchecked_mut(index);
        }
        self.arr.get_unchecked_mut(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub unsafe fn get_0_unchecked_mut(&mut self, index: usize) -> &mut T {
        return self.vec.get_unchecked_mut(index);
    }
    #[inline(always)]
    pub unsafe fn get_1_unchecked_mut(&mut self, index: usize) -> &mut T {
        self.arr.get_unchecked_mut(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub fn load(&self, index: usize) -> Option<&mut T> {
        if index < self.vec.capacity() {
            return Some(self.vec_index_mut(index));
        }
        self.arr.load(index)
    }
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        if index < self.vec.capacity() {
            return self.vec_index_mut(index);
        }
        self.arr.load_unchecked(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub unsafe fn load_0_unchecked(&self, index: usize) -> &mut T {
        return self.vec_index_mut(index);
    }
    #[inline(always)]
    pub unsafe fn load_1_unchecked(&self, index: usize) -> &mut T {
        self.arr.load_unchecked(self.vec.capacity() - index)
    }
    #[inline(always)]
    pub fn load_alloc(&self, index: usize, multiple: usize) -> &mut T {
        if index < self.vec.capacity() {
            return self.vec_index_mut(index);
        }
        self.arr.load_alloc(self.vec.capacity() - index, multiple)
    }
    #[inline(always)]
    fn vec_index_mut(&self, index: usize) -> &mut T {
        unsafe {
            let ptr: *mut T = transmute(self.vec.get_unchecked(index));
            return transmute(ptr);
        }
    }
    #[inline(always)]
    pub fn alloc_index(&self, multiple: usize) -> usize {
        self.len.fetch_add(multiple, Ordering::Relaxed)
    }
    #[inline(always)]
    pub fn insert(&self, value: T) -> usize {
        let index = self.alloc_index(1);
        *self.load_alloc(index, 1) = value;
        index
    }
    #[inline(always)]
    pub fn iter(&self) -> Iter<'_, T> {
        self.slice1(0..self.len())
    }
    #[inline(always)]
    pub fn slice(&self, range: Range<usize>) -> Iter<'_, T> {
        let len = self.len();
        if range.end <= len {
            return self.slice1(range);
        }
        self.slice1(range.start..len)
    }
    #[inline(always)]
    fn slice1(&self, mut range: Range<usize>) -> Iter<'_, T> {
        let c = self.vec.capacity();
        if range.end <= c {
            return self.arr.iter_with_ptr(
                self.vec.as_ptr() as *mut T,
                Location::new(-1, range.end, range.start),
                Location::new(-1, c, range.end),
            );
        } else if range.start < c {
            return self.arr.iter_with_ptr(
                self.vec.as_ptr() as *mut T,
                Location::new(-1, c, range.start),
                Location::of(range.end - c),
            );
        } else {
            range.start -= c;
            range.end -= c;
            return self.arr.slice(range);
        }
    }
    #[inline(always)]
    pub unsafe fn reset(&self) {
        self.len.store(0, Ordering::Relaxed);
    }
    #[inline(always)]
    pub fn vec_capacity(&self) -> usize {
        self.vec.capacity()
    }

    #[inline(always)]
    pub fn collect(&mut self, multiple: usize) {
        let len = mem::take(self.len.get_mut());
        if len <= self.vec.capacity() {
            return;
        }
        let loc = Location::of(len - self.vec.capacity());
        self.vec.reserve(Location::index(loc.bucket as u32 + 1, 0));
        for mut v in self.arr.replace(multiple) {
            self.vec.append(&mut v);
        }
        self.vec.resize_with(self.vec.capacity(), || T::null());
    }

    #[inline(always)]
    pub fn clear(&mut self, multiple: usize) {
        let len = mem::take(self.len.get_mut());
        if len == 0 {
            return;
        }
        self.len.store(0, Ordering::Relaxed);
        self.arr.replace(multiple);
        if needs_drop::<T>() {
            self.vec.clear();
        } else {
            unsafe { self.vec.set_len(0) }
        }
        self.vec.resize_with(len, || T::null());
        unsafe { self.vec.set_len(self.vec_capacity()) }
    }
}
impl<T: Null> Index<usize> for AppendVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("no element found at index {index}")
    }
}
impl<T: Null> IndexMut<usize> for AppendVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index)
            .expect("no element found at index_mut {index}")
    }
}
impl<T: Null + Debug> Debug for AppendVec<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Null> Default for AppendVec<T> {
    fn default() -> Self {
        Self::with_capacity(DEFALLT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test() {
        println!("test: {:?}", 1);
        let mut vec = AppendVec::with_capacity(4);
        let _good_day = vec.insert("Good day");
        let _hello = vec.insert("Hello");
        assert_eq!(vec.len(), 2);
        let hello1 = vec.insert("Hello");
        assert_eq!(vec[hello1], "Hello");
        assert_eq!(unsafe { vec.get_unchecked(hello1) }, &"Hello");
        println!("test: {:?}", 2);
        *vec.get_mut(hello1).unwrap() = "Hello1";
        assert_eq!(vec[hello1], "Hello1");
        assert_eq!(vec.len(), 3);
        println!("test: {:?}", 3);
        println!("vec: {:?}", vec);
    }
}
