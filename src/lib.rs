//! 兼顾性能和安全的线程安全的vec
//! 使用一个vec，加线程安全的pi_arr
//! 正常使用时， vec的内存不会扩大，放不下的数据会放到pi_arr上
//! 整理方法settle，要求必须mut引用，这时会安全的vec先扩容，然后将pi_arr的数据移动到vec上

use core::fmt::*;
use std::mem::take;
use std::ops::{Index, IndexMut, Range};
use std::sync::atomic::Ordering;

use pi_arr::{Arr, Iter};
use pi_null::Null;
use pi_share::ShareUsize;

pub struct AppendVec<T: Null> {
    len: ShareUsize,
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
        Self {
            len: ShareUsize::new(0),
            arr: Arr::with_capacity(capacity),
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
        self.arr.get(index)
    }
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        self.arr.get_unchecked(index)
    }
    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let len = *self.len.get_mut();
        if index >= len {
            return None;
        }
        self.arr.get_mut(index)
    }

    #[inline(always)]
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        self.arr.get_unchecked_mut(index)
    }
    #[inline(always)]
    pub fn load(&self, index: usize) -> Option<&mut T> {
        if index >= self.len() {
            return None;
        }
        self.arr.load(index)
    }
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        self.arr.load_unchecked(index)
    }
    #[inline(always)]
    pub fn alloc(&self) -> (&mut T, usize) {
        let index = self.len.fetch_add(1, Ordering::Relaxed);
        (self.arr.load_alloc(index), index)
    }
    #[inline(always)]
    pub fn alloc_index(&self, multiple: usize) -> usize {
        self.len.fetch_add(multiple, Ordering::Relaxed)
    }
    #[inline(always)]
    pub fn insert(&self, value: T) -> usize {
        let index = self.alloc_index(1);
        *self.arr.load_alloc(index) = value;
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
    pub fn slice_raw(&self, range: Range<usize>) -> Iter<'_, T> {
        self.arr.slice(range)
    }
    #[inline(always)]
    pub unsafe fn set_len(&self, len: usize) {
        self.len.store(len, Ordering::Relaxed);
    }
    #[inline(always)]
    pub fn vec_capacity(&self) -> usize {
        self.arr.vec_capacity()
    }
    /// 将arr的内容移动到vec上，让内存连续，并且没有原子操作
    #[inline(always)]
    pub fn settle(&mut self, additional: usize) {
        let len = self.len();
        if len == 0 {
            return;
        }
        self.arr.settle(len, additional, 1);
    }
    /// 清理，并释放arr的内存
    #[inline(always)]
    pub fn clear(&mut self, additional: usize) {
        let len = take(self.len.get_mut());
        if len == 0 {
            return;
        }
        self.arr.clear(len, additional, 1);
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
        println!("test start");
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
        removes.clear(1);
        removes.insert(1);
        removes.insert(6);
    }
}
