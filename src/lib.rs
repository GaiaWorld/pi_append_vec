use core::fmt::*;
use std::ops::{Index, IndexMut};
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
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            arr: Arr::with_capacity(capacity),
            max: ShareUsize::new(0),
            inserting: ShareUsize::new(0),
        }
    }
    /// 长度
    pub fn len(&self) -> usize {
        self.max.load(Ordering::Acquire)
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        self.arr.get(index)
    }
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        self.arr.get_unchecked(index)
    }
    pub fn set(&mut self, value: T) -> usize {
        let index = self.inserting.fetch_add(1, Ordering::AcqRel);
        let i = self.arr.get_alloc(index);
        *i = value;
        self.max.store(index, Ordering::Release);
        index
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.arr.get_mut(index)
    }
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        self.arr.get_unchecked_mut(index)
    }
    pub fn insert(&self, value: T) -> usize {
        let index = self.inserting.fetch_add(1, Ordering::AcqRel);
        let i = self.arr.load_alloc(index);
        *i = value;
        while self
            .max
            .compare_exchange(index, index + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {}
        index
    }
    pub fn iter(&self) -> Iter<'_, T> {
        self.arr.slice(0..self.len())
    }
    pub unsafe fn clear(&self) {
        self.inserting.store(0, Ordering::Release);
        self.max.store(0, Ordering::Release);
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
        assert_eq!(vec.len(), 3);
        println!("vec: {:?}", vec);
    }
}
