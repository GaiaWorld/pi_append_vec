//! 线程安全的仅添加vec，依赖整理方法保证内存连续
//! AppendVec保证用index访问时，是对象可见和线程安全的
//! SafeVec保证iter访问时，是对象可见和线程安全的
//! 整理方法settle，要求必须mut引用，这时会安全的进行内存整理

use core::fmt::*;
use std::mem::{needs_drop, take, transmute, MaybeUninit};
use std::ops::{Index, IndexMut, Range};
use std::sync::atomic::Ordering;

use pi_arr::{Arr, Iter};
use pi_share::ShareUsize;

pub struct AppendVec<T> {
    len: ShareUsize,
    arr: Arr<T>,
}
impl<T: Default> AppendVec<T> {
    /// Creates an empty [`AppendVec`] with the given capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use crate::pi_append_vec::AppendVec;
    /// let mut vec: AppendVec<&str> = AppendVec::with_capacity(3);
    /// let welcome = vec.insert("Welcome");
    /// let good_day = vec.insert("Good day");
    /// let hello = vec.insert("Hello");
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
        Some(self.arr.load_alloc(index))
    }
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        self.arr.load_alloc(index)
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
    pub fn capacity(&self) -> usize {
        self.arr.capacity(self.len())
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
impl<T: Default> Index<usize> for AppendVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("no element found at index {index}")
    }
}
impl<T: Default> IndexMut<usize> for AppendVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index)
            .expect("no element found at index_mut {index}")
    }
}
impl<T: Default + Debug> Debug for AppendVec<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Default> Default for AppendVec<T> {
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

pub struct SafeVec<T> {
    vec: AppendVec<Element<T>>,
    len: ShareUsize,
}
impl<T> SafeVec<T> {
    #[inline(always)]
    pub fn with_capacity(capacity: usize) -> Self {
        let vec = AppendVec::with_capacity(capacity);
        Self {
            vec,
            len: ShareUsize::new(0),
        }
    }
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.vec.arr.capacity(self.len())
    }
    /// 长度
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&T> {
        let len = self.len();
        if index >= len {
            return None;
        }
        self.vec.get(index).map(|r| unsafe { &*r.0.as_ptr() })
    }
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        &*self.vec.get_unchecked(index).0.as_ptr()
    }
    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let len = self.len();
        if index >= len {
            return None;
        }
        self.vec
            .get_mut(index)
            .map(|r| unsafe { &mut *r.0.as_mut_ptr() })
    }
    #[inline(always)]
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        &mut *self.vec.get_unchecked_mut(index).0.as_mut_ptr()
    }
    #[inline(always)]
    pub fn load(&self, index: usize) -> Option<&mut T> {
        self.vec
            .load(index)
            .map(|r| unsafe { &mut *r.0.as_mut_ptr() })
    }
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        &mut *self.vec.load_unchecked(index).0.as_mut_ptr()
    }

    #[inline(always)]
    pub fn insert(&self, value: T) -> usize {
        let (r, index) = self.vec.alloc();
        *r = Element(MaybeUninit::new(value));
        while self
            .len
            .compare_exchange(index, index + 1, Ordering::Release, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        index
    }
    #[inline(always)]
    pub fn alloc_entry<'a>(&'a self) -> Entry<'a, T> {
        let (value, index) = self.vec.alloc();
        Entry {
            index,
            len: &self.len,
            value,
        }
    }
    #[inline(always)]
    pub fn iter(&self) -> SafeVecIter<'_, T> {
        SafeVecIter(self.vec.slice(0..self.len()))
    }
    #[inline(always)]
    pub fn slice(&self, range: Range<usize>) -> SafeVecIter<'_, T> {
        SafeVecIter(self.vec.slice(range))
    }
    pub fn vec_capacity(&self) -> usize {
        self.vec.vec_capacity()
    }
    #[inline(always)]
    pub fn settle(&mut self, additional: usize) {
        self.vec.settle(additional);
    }

    #[inline(always)]
    pub fn clear(&mut self, additional: usize) {
        let len = take(self.len.get_mut());
        if len == 0 {
            return;
        }
        if needs_drop::<T>() {
            for i in self.vec.iter() {
                unsafe { i.0.assume_init_drop() }
            }
        }
        self.vec.clear(additional);
    }
}
impl<T> Index<usize> for SafeVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("no element found at index {index}")
    }
}
impl<T> IndexMut<usize> for SafeVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index)
            .expect("no element found at index_mut {index}")
    }
}
impl<T> Drop for SafeVec<T> {
    fn drop(&mut self) {
        if needs_drop::<T>() {
            for i in self.vec.iter() {
                unsafe { i.0.assume_init_drop() }
            }
        }
    }
}
impl<T> Default for SafeVec<T> {
    fn default() -> Self {
        SafeVec {
            vec: Default::default(),
            len: ShareUsize::new(0),
        }
    }
}
impl<T: Debug> Debug for SafeVec<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

struct Element<T>(MaybeUninit<T>);
impl<T> Default for Element<T> {
    fn default() -> Self {
        Self(MaybeUninit::uninit())
    }
}

pub struct SafeVecIter<'a, T>(Iter<'a, Element<T>>);
impl<'a, T> Iterator for SafeVecIter<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|r| unsafe { transmute(r.0.as_ptr()) })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

pub struct Entry<'a, T> {
    index: usize,
    len: &'a ShareUsize,
    value: &'a mut Element<T>,
}
impl<'a, T> Entry<'_, T> {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn insert(self, value: T) {
        *self.value = Element(MaybeUninit::new(value));
    }
}
impl<'a, T> Drop for Entry<'_, T> {
    fn drop(&mut self) {
        while self
            .len
            .compare_exchange(
                self.index,
                self.index + 1,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {
            std::hint::spin_loop();
        }
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

    #[test]
    fn test_str() {
        let vec = AppendVec::with_capacity(4);
        let _good_day = vec.insert("Good day");
        let _hello = vec.insert("Hello");
        assert_eq!(vec.len(), 2);
        let hello1 = vec.insert("Hello");
        assert_eq!(vec[hello1], "Hello");
    }
}
