//! 兼顾性能和安全的线程安全的vec
//! 使用一个vec，加线程安全的pi_arr
//! 正常使用时， vec的内存不会扩大，放不下的数据会放到pi_arr上
//! 整理方法settle，要求必须mut引用，这时会安全的vec先扩容，然后将pi_arr的数据移动到vec上

use core::fmt::*;
use std::mem::{replace, size_of, take, transmute};
use std::ops::{Index, IndexMut, Range};
use std::ptr::null_mut;
use std::sync::atomic::Ordering;

use pi_arr::*;
use pi_null::Null;
use pi_share::ShareUsize;

extern crate pi_arr;

pub struct AppendVec<T: Null> {
    len: ShareUsize,
    vec: Vec<T>,
    arr: Arr<T>,
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
        let mut vec = Vec::with_capacity(capacity);
        if size_of::<T>() > 0 {
            vec.resize_with(vec.capacity(), || T::null());
        }
        Self {
            len: ShareUsize::new(0),
            vec,
            arr: Arr::new(),
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
        self.get_i(index)
    }
    #[inline(always)]
    pub fn get_i(&self, index: usize) -> Option<&T> {
        if index < self.vec.capacity() {
            return Some(unsafe { self.vec.get_unchecked(index) });
        }
        self.arr.get(&Location::of(index - self.vec.capacity()))
    }
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        if index < self.vec.capacity() {
            return unsafe { self.vec.get_unchecked(index) };
        }
        self.arr
            .get_unchecked(&Location::of(index - self.vec.capacity()))
    }
    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let len = *self.len.get_mut();
        if index >= len {
            return None;
        }
        self.get_mut_i(index)
    }
    #[inline(always)]
    pub fn get_mut_i(&mut self, index: usize) -> Option<&mut T> {
        if index < self.vec.capacity() {
            return self.vec.get_mut(index);
        }
        self.arr.get_mut(&Location::of(index - self.vec.capacity()))
    }

    #[inline(always)]
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        if index < self.vec.capacity() {
            return self.vec.get_unchecked_mut(index);
        }
        self.arr
            .get_unchecked_mut(&Location::of(index - self.vec.capacity()))
    }
    #[inline(always)]
    pub fn load(&self, index: usize) -> Option<&mut T> {
        if index >= self.len() {
            return None;
        }
        self.load_i(index)
    }
    #[inline(always)]
    pub fn load_i(&self, index: usize) -> Option<&mut T> {
        if index < self.vec.capacity() {
            return Some(self.vec_index_mut(index));
        }
        self.arr.load(&Location::of(index - self.vec.capacity()))
    }
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        if index < self.vec.capacity() {
            return self.vec_index_mut(index);
        }
        self.arr
            .load_unchecked(&Location::of(index - self.vec.capacity()))
    }
    #[inline(always)]
    pub fn load_alloc(&self, index: usize) -> &mut T {
        if index < self.vec.capacity() {
            return self.vec_index_mut(index);
        }
        self.arr
            .load_alloc(&Location::of(index - self.vec.capacity()))
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
        *self.load_alloc(index) = value;
        index
    }
    #[inline(always)]
    pub fn iter(&self) -> Iter<'_, T> {
        self.slice_raw(0..self.len())
    }
    #[inline(always)]
    pub fn slice(&self, range: Range<usize>) -> Iter<'_, T> {
        let len = self.len();
        if range.end <= len {
            return self.slice_raw(range);
        }
        self.slice_raw(range.start..len)
    }
    #[inline(always)]
    pub fn slice_raw(&self, mut range: Range<usize>) -> Iter<'_, T> {
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
    pub unsafe fn set_len(&self, len: usize) {
        self.len.store(len, Ordering::Relaxed);
    }
    #[inline(always)]
    pub fn vec_capacity(&self) -> usize {
        self.vec.capacity()
    }
    #[inline(always)]
    pub unsafe fn vec_reserve(&mut self, additional: usize) {
        if size_of::<T>() == 0 {
            return
        }
        self.vec.reserve(additional);
        self.vec.resize_with(self.vec.capacity(), || T::null());
    }
    /// reserve capacity
    pub fn reserve(&mut self, additional: usize) {
        let len = self.len();
        if len + additional <= self.vec.capacity() {
            return;
        }
        self.settle_raw(len, additional)
    }
    /// 将arr的内容移动到vec上，让内存连续，并且没有原子操作
    #[inline(always)]
    pub fn settle(&mut self) {
        let len = self.len();
        if len <= self.vec.capacity() {
            return;
        }
        self.settle_raw(len, 0)
    }
    #[inline(always)]
    pub fn settle_raw(&mut self, len: usize, additional: usize) {
        if size_of::<T>() == 0 {
            return
        }
        if len <= self.vec.capacity() {
            return unsafe { self.vec_reserve(additional) };
        }
        let loc = Location::of(len - self.vec.capacity());
        let mut len = Location::index(loc.bucket as u32 + 1, 0);
        let mut arr = Self::replace(self.arr.replace());
        if self.vec.capacity() == 0 {
            // 如果原vec为empty，则直接将arr的0位vec换上
            len = len.saturating_sub(arr[0].len());
            let _ = replace(&mut self.vec, take(&mut arr[0]));
        }
        // 将vec扩容
        self.vec.reserve(len + additional);
        for mut v in arr.into_iter() {
            len = len.saturating_sub(v.len());
            self.vec.append(&mut v);
            if len == 0 {
                break;
            }
        }
        // 如果容量比len大，则初始化为null元素
        self.vec.resize_with(self.vec.capacity(), || T::null());
    }
    /// 清理，并释放arr的内存
    #[inline(always)]
    pub fn clear(&mut self) {
        let len = take(self.len.get_mut());
        if len == 0 {
            return;
        }
        if len > self.vec.capacity() {
            let _ = Self::replace(self.arr.replace());
        }
        self.vec.clear();
    }
    fn replace(arr: [*mut T; BUCKETS]) -> [Vec<T>; BUCKETS] {
        let mut buckets = [0; BUCKETS].map(|_| Vec::new());
        for (i, p) in arr.iter().enumerate() {
            if *p != null_mut() {
                let len = Location::bucket_len(i);
                buckets[i] = unsafe { Vec::from_raw_parts(*p, len, len) };
            }
        }
        buckets
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
        Self::with_capacity(0)
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
    #[test]
    fn test_removes() {
        let mut removes: AppendVec<usize> = Default::default();
        removes.insert(1);
        removes.insert(2);
        removes.clear();
        removes.insert(1);
        removes.insert(6);
    }
}
