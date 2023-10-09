use core::fmt::*;
use core::mem::ManuallyDrop;
use std::ops::{Index, IndexMut};
use std::sync::atomic::Ordering;

use pi_arr::*;
use pi_null::Null;
use pi_share::{ShareBool, ShareUsize};

#[derive(Default)]
pub struct AppendVec<T> {
    arr: Arr<Slot<T>>,
    max: ShareUsize,
}
impl<T> AppendVec<T> {
    /// Creates an empty [`SlotMap`] with the given capacity and a custom key
    /// type.
    ///
    /// The slot map will not reallocate until it holds at least `capacity`
    /// elements.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pi_slot::*;
    /// new_key_type! {
    ///     struct MessageKey;
    /// }
    /// let mut messages = SlotMap::with_capacity_and_key(3);
    /// let welcome: MessageKey = messages.insert("Welcome");
    /// let good_day = messages.insert("Good day");
    /// let hello = messages.insert("Hello");
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            arr: Arr::with_capacity(capacity),
            max: ShareUsize::new(0),
        }
    }
    /// 长度
    pub fn len(&self) -> usize {
        self.max.load(Ordering::Acquire)
    }
    pub fn get(&self, index: usize) -> Option<&T> {
        self.arr.get(index).map_or(None, |i| i.get())
    }
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        self.arr.get_unchecked(index).get_unchecked()
    }
    pub fn set(&mut self, value: T) -> usize {
        let index = self.max.fetch_add(1, Ordering::AcqRel);
        let i = self.arr.get_alloc(index);
        i.value.value = ManuallyDrop::new(value);
        i.set_used();
        index
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.arr.get_mut(index).map_or(None, |i| i.get_mut())
    }
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        self.arr.get_unchecked_mut(index).get_unchecked_mut()
    }
    pub fn insert(&self, value: T) -> usize {
        let index = self.max.fetch_add(1, Ordering::AcqRel);
        let i = self.arr.load_alloc(index);
        i.value.value = ManuallyDrop::new(value);
        i.set_used();
        index
    }
    pub fn iter(&self) -> Iter<'_, T> {
        let max = self.len();
        Iter {
            iter: self.arr.slice(0..max),
            max,
        }
    }
}
impl<T> Index<usize> for AppendVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        let i = &self.arr[index];
        if i.is_null() {
            panic!("no element found at index {}", index)
        }
        unsafe { i.get_unchecked() }
    }
}
impl<T> IndexMut<usize> for AppendVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let i = &mut self.arr[index];
        if i.is_null() {
            panic!("no element found at index {}", index)
        }
        unsafe { i.get_unchecked_mut() }
    }
}
impl<T: Debug> Debug for AppendVec<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

union SlotUnion<T> {
    none: (),
    value: ManuallyDrop<T>,
}
struct Slot<T> {
    value: SlotUnion<T>,
    used: ShareBool,
}
impl<T> Slot<T> {
    #[inline]
    pub fn get(&self) -> Option<&T> {
        if self.is_null() {
            None
        } else {
            unsafe { Some(&self.value.value) }
        }
    }
    #[inline]
    pub unsafe fn get_unchecked(&self) -> &T {
        &self.value.value
    }
    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        if self.is_null() {
            None
        } else {
            unsafe { Some(&mut self.value.value) }
        }
    }
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self) -> &mut T {
        &mut self.value.value
    }
    fn set_used(&mut self) {
        self.used.store(true, Ordering::Release)
    }
}

impl<T> Drop for Slot<T> {
    fn drop(&mut self) {
        if core::mem::needs_drop::<T>() && !self.is_null() {
            // This is safe because we checked that we're not null.
            unsafe {
                ManuallyDrop::drop(&mut self.value.value);
            }
        }
    }
}
impl<T> Null for Slot<T> {
    fn null() -> Self {
        Self {
            value: SlotUnion { none: () },
            used: ShareBool::new(false),
        }
    }
    fn is_null(&self) -> bool {
        !self.used.load(Ordering::Acquire)
    }
}
impl<T> Default for Slot<T> {
    #[inline]
    fn default() -> Self {
        Self::null()
    }
}

pub struct Iter<'a, T> {
    iter: pi_arr::Iter<'a, Slot<T>>,
    max: usize,
}
impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((_, s)) = self.iter.next() {
            if s.is_null() {
                return None;
            }
            self.max -= 1;
            return Some(unsafe { &mut s.value.value });
        }
        None
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.max))
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
