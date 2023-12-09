use core::fmt::*;
use std::ops::{Index, IndexMut, Range};
use std::sync::atomic::Ordering;

use pi_arr::*;
use pi_null::Null;
use pi_share::ShareUsize;

pub struct AppendVec<T: Null> {
    arr: Arr<T>,
    max: ShareUsize,
    inserting: ShareUsize,
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
            arr: Arr::with_capacity(capacity),
            max: ShareUsize::new(0),
            inserting: ShareUsize::new(0),
        }
    }
    /// 长度
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.max.load(Ordering::Acquire)
    }
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.arr.get(index)
    }
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        self.arr.get_unchecked(index)
    }
    #[inline(always)]
    pub fn set(&mut self, value: T) -> usize {
        let index = self.inserting.fetch_add(1, Ordering::Relaxed);
        let i = self.arr.get_alloc(index);
        *i = value;
        self.max.store(index, Ordering::Release);
        index
    }
    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.arr.get_mut(index)
    }
    #[inline(always)]
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        self.arr.get_unchecked_mut(index)
    }
    #[inline(always)]
    pub fn load(&self, index: usize) -> Option<&mut T> {
        self.arr.load(index)
    }
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        self.arr.load_unchecked(index)
    }
    #[inline(always)]
    pub fn insert_entry<'a>(&'a self) -> Entry<'a, T> {
        let index = self.inserting.fetch_add(1, Ordering::Relaxed);
        Entry {
            index,
            max: &self.max,
            value: self.arr.load_alloc(index),
        }
    }
    #[inline(always)]
    pub fn insert(&self, value: T) -> usize {
        let index = self.inserting.fetch_add(1, Ordering::Relaxed);
        let i = self.arr.load_alloc(index);
        *i = value;
        while self
            .max
            .compare_exchange(index, index + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {}
        index
    }
    #[inline(always)]
    pub fn slice(&self, range: Range<usize>) -> Iter<'_, T> {
        self.arr.slice(range)
    }
    #[inline(always)]
    pub fn iter(&self) -> Iter<'_, T> {
        self.arr.slice(0..self.len())
    }
    #[inline(always)]
    pub unsafe fn reset(&self) {
        self.inserting.store(0, Ordering::Relaxed);
        self.max.store(0, Ordering::Release);
    }
    #[inline(always)]
    pub unsafe fn clear(&self) {
        self.reset();
        self.arr.clear();
    }
}
impl<T: Null> Index<usize> for AppendVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.arr[index]
    }
}
impl<T: Null> IndexMut<usize> for AppendVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.arr[index]
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

pub struct Entry<'a, T> {
    index: usize,
    max: &'a ShareUsize,
    pub value: &'a mut T,
}
impl<'a, T> Entry<'_, T> {
    pub fn index(&self) -> usize {
        self.index
    }
}
impl<'a, T> Drop for Entry<'_, T> {
    fn drop(&mut self) {
        while self
            .max
            .compare_exchange(
                self.index,
                self.index + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {}
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test() {
        let mut vec = AppendVec::with_capacity(3);
        let _good_day = vec.insert("Good day");
        let _hello = vec.insert("Hello");
        assert_eq!(vec.len(), 2);
        let hello1 = vec.insert("Hello");
        assert_eq!(vec[hello1], "Hello");
        assert_eq!(unsafe { vec.get_unchecked(hello1) }, &"Hello");
        *vec.get_mut(hello1).unwrap() = "Hello1";
        assert_eq!(vec[hello1], "Hello1");
        {
            let e = vec.insert_entry();
            *e.value = "Hello2";
        }
        assert_eq!(vec.len(), 4);
        println!("vec: {:?}", vec);
    }
}
