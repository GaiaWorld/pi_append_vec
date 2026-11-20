use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn panics(operation: impl FnOnce()) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).is_err()
    }

    #[test]
    fn failure_scope_restores_after_unwind() {
        assert!(panics(|| with_failure(
            FailurePoint::AppendBeforeAllocation,
            || panic!("operation")
        )));
        with_failure(FailurePoint::RebuildBeforeAllocation, || ());
    }

    #[test]
    fn failure_checkpoints_are_nested_and_thread_local() {
        fail_if_requested(FailurePoint::AppendBeforeAllocation);
        with_failure(FailurePoint::AppendBeforeAllocation, || {
            fail_if_requested(FailurePoint::AppendAfterPreparation);
            with_failure(FailurePoint::RebuildBeforeAllocation, || {
                assert!(panics(|| fail_if_requested(
                    FailurePoint::RebuildBeforeAllocation
                )));
            });
            std::thread::spawn(|| fail_if_requested(FailurePoint::AppendBeforeAllocation))
                .join()
                .unwrap();
            assert!(panics(|| fail_if_requested(
                FailurePoint::AppendBeforeAllocation
            )));
        });
        fail_if_requested(FailurePoint::AppendBeforeAllocation);
    }

    #[test]
    fn empty_reclamation_guard_needs_no_allocation() {
        let mut storage = SegmentedArr::<Slot<String>>::default();
        // SAFETY: both ranges are empty and storage remains alive.
        let guard = unsafe { RemovedValues::new(&mut storage, [0..0, 0..0]) };
        let mut guard = std::mem::ManuallyDrop::new(guard);
        guard.drain();
        guard.drain();
    }

    #[test]
    fn construction_has_exact_spare_slots() {
        let values = std::mem::ManuallyDrop::new(AppendVec::<std::rc::Rc<()>>::with_capacity(5));
        assert_eq!(values.len(), 0);
        assert!(values.is_empty());
        assert_eq!(values.contiguous_slots(), 5);
    }

    #[test]
    fn empty_container_drop_ignores_spare_slots() {
        let values = AppendVec::<String>::with_capacity(5);
        drop(values);
        drop(AppendVec::<String>::default());
    }

    #[test]
    fn push_returns_published_dense_indices() {
        let values = AppendVec::with_capacity(1);
        for n in 0..100 {
            assert_eq!(values.push(n), n);
            assert_eq!(values.len(), n + 1);
        }
    }

    #[test]
    fn unchecked_shared_reads_initialized_values() {
        let values = AppendVec::with_capacity(1);
        values.push(String::from("first"));
        values.push(String::from("bucket"));
        assert_eq!(values.len(), 2);
        unsafe {
            assert_eq!(values.get_unchecked(0), "first");
            assert_eq!(values.get_unchecked(1), "bucket");
        }
    }

    #[test]
    fn get_excludes_unpublished_spare_slots() {
        let values = AppendVec::with_capacity(8);
        assert_eq!(values.get(0), None);
        values.push(42);
        assert_eq!(values.get(0), Some(&42));
        assert_eq!(values.get(1), None);
        assert_eq!(values.get(usize::MAX), None);
    }

    #[test]
    fn unchecked_mutation_in_prefix_and_bucket() {
        let mut values = AppendVec::with_capacity(1);
        values.push(10);
        values.push(20);
        unsafe {
            *values.get_unchecked_mut(0) += 1;
            *values.get_unchecked_mut(1) += 2;
        }
        assert_eq!(values.get(0), Some(&11));
        assert_eq!(values.get(1), Some(&22));
    }

    #[test]
    fn checked_mutation_rejects_spare_slots() {
        let mut values = AppendVec::with_capacity(4);
        values.push(10);
        *values.get_mut(0).unwrap() = 30;
        assert_eq!(values.get(0), Some(&30));
        assert!(values.get_mut(1).is_none());
        assert!(values.get_mut(usize::MAX).is_none());
        values[0] = 40;
        assert_eq!(values[0], 40);
    }

    #[test]
    fn unsafe_shared_receiver_projects_only_target_value() {
        let values = AppendVec::with_capacity(1);
        values.push(1);
        values.push(2);
        assert_eq!(values.len(), 2);
        let first = values.get(0).unwrap();
        unsafe {
            *values.load_unchecked(1) = 7;
        }
        assert_eq!(*first, 1);
        assert_eq!(values.get(1), Some(&7));
    }

    #[test]
    fn unsafe_checked_load_excludes_spare_slots() {
        let values = AppendVec::with_capacity(4);
        values.push(5);
        unsafe {
            *values.load(0).unwrap() = 8;
            assert!(values.load(1).is_none());
            assert!(values.load(usize::MAX).is_none());
        }
        assert_eq!(values.get(0), Some(&8));
    }

    #[test]
    fn shared_cursor_projects_initialized_slots() {
        let values = AppendVec::with_capacity(1);
        values.push(3);
        values.push(4);
        let mut cursor = SharedIter {
            slots: values.storage.slice(0..values.len()),
        };
        assert_eq!(cursor.next(), Some(&3));
        assert_eq!(cursor.next(), Some(&4));
        assert_eq!(cursor.next(), None);
        assert_eq!(cursor.next(), None);
    }

    #[test]
    fn iteration_keeps_a_published_snapshot() {
        let values = AppendVec::with_capacity(1);
        values.push(3);
        values.push(4);
        let snapshot = values.iter();
        values.push(5);
        assert_eq!(snapshot.copied().collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(format!("{values:?}"), "[3, 4, 5]");
    }

    #[test]
    fn exclusive_cursor_returns_disjoint_references() {
        let mut values = AppendVec::with_capacity(1);
        values.push(3);
        values.push(4);
        let len = values.len();
        let mut cursor = ExclusiveIter {
            slots: values.storage.slice_mut(0..len),
        };
        let a = cursor.next().unwrap();
        let b = cursor.next().unwrap();
        *a = 7;
        *b = 8;
        assert!(cursor.next().is_none());
        assert_eq!(values.iter().copied().collect::<Vec<_>>(), vec![7, 8]);
    }

    #[test]
    fn mutable_iteration_visits_only_published_values() {
        let mut values = AppendVec::with_capacity(1);
        for n in 0..70 {
            values.push(n);
        }
        let refs = values.iter_mut().collect::<Vec<_>>();
        for value in refs {
            *value += 1;
        }
        assert_eq!(values.get(0), Some(&1));
        assert_eq!(values.get(69), Some(&70));
        assert_eq!(values.iter().count(), 70);
    }

    #[test]
    fn slice_checks_strict_ranges_and_snapshot() {
        let values = AppendVec::with_capacity(2);
        for n in 0..70 {
            values.push(n);
        }
        let slice = values.slice(1..4);
        values.push(70);
        assert_eq!(slice.copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(values.slice(0..0).count(), 0);
        assert_eq!(values.slice(71..71).count(), 0);
        for range in [4..3, 0..72, usize::MAX..usize::MAX] {
            assert!(panics(|| {
                values.slice(range).count();
            }));
        }
    }

    #[test]
    fn allocated_slots_counts_whole_buckets_and_spares() {
        let values = AppendVec::with_capacity(3);
        assert_eq!(values.allocated_slots(), 3);
        for n in 0..4 {
            values.push(n);
        }
        assert_eq!(values.allocated_slots(), 35);
        for n in 4..36 {
            values.push(n);
        }
        assert_eq!(values.allocated_slots(), 99);
        assert_eq!(AppendVec::<()>::with_capacity(7).allocated_slots(), 7);
    }

    #[test]
    fn retain_installs_ordered_exact_result() {
        let mut values = AppendVec::with_capacity(2);
        for n in 0..100 {
            values.push(n.to_string());
        }
        values.retain_range_and_compact(30..36, 3);
        assert_eq!(
            values.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["30", "31", "32", "33", "34", "35"]
        );
        assert_eq!(values.len(), 6);
        assert_eq!(values.contiguous_slots(), 9);
        assert_eq!(values.allocated_slots(), 9);
        assert_eq!(values.push("next".into()), 6);
    }

    #[test]
    fn make_contiguous_rebuilds_exact_capacity() {
        let mut values = AppendVec::with_capacity(150);
        for n in 0..100 {
            values.push(n);
        }
        values.make_contiguous(2);
        assert_eq!(values.len(), 100);
        assert_eq!(values.allocated_slots(), 102);
        assert_eq!(values.contiguous_slots(), 102);
        assert_eq!(
            values.iter().copied().collect::<Vec<_>>(),
            (0..100).collect::<Vec<_>>()
        );
        values.make_contiguous(0);
        assert_eq!(values.allocated_slots(), 100);
    }

    #[test]
    fn clear_replaces_capacity_and_allows_reuse() {
        let mut values = AppendVec::with_capacity(2);
        for n in 0..100 {
            values.push(n);
        }
        values.clear(3);
        assert!(values.is_empty());
        assert_eq!(values.contiguous_slots(), 3);
        assert_eq!(values.allocated_slots(), 3);
        assert_eq!(values.push(8), 0);
        values.clear(0);
        assert_eq!(values.allocated_slots(), 0);
        assert_eq!(values.push(9), 0);
    }

    #[test]
    fn append_preparation_failures_unlock_before_value_drop_and_recover() {
        struct CheckUnlock<'a> {
            lock: *const Mutex<()>,
            dropped: &'a std::cell::Cell<bool>,
        }
        impl Drop for CheckUnlock<'_> {
            fn drop(&mut self) {
                // A poisoned but acquired guard is success; WouldBlock means the lock is held.
                assert!(!matches!(
                    unsafe { &*self.lock }.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                self.dropped.set(true);
            }
        }
        for point in [
            FailurePoint::AppendBeforeAllocation,
            FailurePoint::AppendAfterPreparation,
        ] {
            let values = AppendVec::with_capacity(0);
            values.push(10);
            let old_capacity = values.allocated_slots();
            assert!(panics(|| with_failure(point, || {
                values.push(20);
            })));
            assert_eq!(values.len(), 1);
            assert_eq!(values.get(0), Some(&10));
            assert_eq!(values.allocated_slots(), old_capacity);
            assert!(values.commit_lock.is_poisoned());
            assert_eq!(values.push(30), 1);
            assert!(!values.commit_lock.is_poisoned());
            assert_eq!(values.get(1), Some(&30));

            let observed = std::cell::Cell::new(false);
            let failing = AppendVec::<CheckUnlock<'_>>::default();
            // The container stays in place and alive until after the injected unwind.
            let lock = &failing.commit_lock as *const Mutex<()>;
            assert!(panics(|| with_failure(point, || {
                failing.push(CheckUnlock {
                    lock,
                    dropped: &observed,
                });
            })));
            assert!(observed.get());
            assert_eq!(failing.len(), 0);
            assert_eq!(
                failing.allocated_slots(),
                if point == FailurePoint::AppendBeforeAllocation {
                    0
                } else {
                    pi_buckets::SKIP
                }
            );
        }
    }

    #[test]
    fn rebuild_failures_preserve_all_values_addresses_and_capacity() {
        for point in [
            FailurePoint::RebuildBeforeAllocation,
            FailurePoint::RebuildAfterPreparation,
        ] {
            for operation in 0..3 {
                let mut values = AppendVec::with_capacity(2);
                for n in 0..100 {
                    values.push(n.to_string());
                }
                let address = values.get(0).unwrap() as *const String;
                let old_capacity = values.allocated_slots();
                assert!(panics(|| with_failure(point, || match operation {
                    0 => values.make_contiguous(3),
                    1 => values.retain_range_and_compact(20..40, 3),
                    _ => values.clear(3),
                })));
                assert_eq!(values.len(), 100);
                assert_eq!(values.contiguous_slots(), 2);
                assert_eq!(values.allocated_slots(), old_capacity);
                assert_eq!(values.get(0).unwrap() as *const String, address);
                for n in 0..100 {
                    assert_eq!(values.get(n).unwrap(), &n.to_string());
                }
                assert_eq!(values.push("next".into()), 100);
            }
        }
    }

    #[test]
    fn invalid_rebuild_ranges_and_capacity_overflow_are_transactional() {
        let mut values = AppendVec::with_capacity(2);
        values.push(String::from("a"));
        values.push(String::from("b"));
        for range in [2..1, 0..3, usize::MAX..usize::MAX] {
            assert!(panics(|| values.retain_range_and_compact(range, 0)));
        }
        assert!(panics(|| values.make_contiguous(usize::MAX)));
        assert!(panics(|| values.retain_range_and_compact(0..1, usize::MAX)));
        assert_eq!(
            values.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(values.allocated_slots(), 2);
    }

    struct DropProbe {
        id: usize,
        counts: std::sync::Arc<Vec<AtomicUsize>>,
        panic_at: Option<usize>,
    }
    impl Drop for DropProbe {
        fn drop(&mut self) {
            assert_eq!(self.counts[self.id].fetch_add(1, Ordering::SeqCst), 0);
            if self.panic_at == Some(self.id) {
                panic!("user destructor");
            }
        }
    }
    fn probes(panic_at: Option<usize>) -> (AppendVec<DropProbe>, std::sync::Arc<Vec<AtomicUsize>>) {
        let counts = std::sync::Arc::new((0..100).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let values = AppendVec::with_capacity(2);
        for id in 0..100 {
            values.push(DropProbe {
                id,
                counts: counts.clone(),
                panic_at,
            });
        }
        (values, counts)
    }

    #[test]
    fn drop_panic_resumes_both_removed_ranges_after_installation() {
        for panic_at in [0, 70] {
            let (mut values, counts) = probes(Some(panic_at));
            assert!(panics(|| values.retain_range_and_compact(30..60, 2)));
            assert_eq!(values.len(), 30);
            assert_eq!(values.contiguous_slots(), 32);
            assert_eq!(values.allocated_slots(), 32);
            assert_eq!(
                values.iter().map(|v| v.id).collect::<Vec<_>>(),
                (30..60).collect::<Vec<_>>()
            );
            for id in 0..100 {
                assert_eq!(
                    counts[id].load(Ordering::SeqCst),
                    usize::from(!(30..60).contains(&id))
                );
            }
            assert_eq!(
                values.push(DropProbe {
                    id: 0,
                    counts: std::sync::Arc::new((0..1).map(|_| AtomicUsize::new(0)).collect()),
                    panic_at: None
                }),
                30
            );
            drop(values);
            for count in counts.iter() {
                assert_eq!(count.load(Ordering::SeqCst), 1);
            }
        }
    }

    #[test]
    fn clear_and_container_drop_resume_after_user_panic() {
        let (mut values, counts) = probes(Some(0));
        assert!(panics(|| values.clear(0)));
        assert!(values.is_empty());
        assert_eq!(values.allocated_slots(), 0);
        for count in counts.iter() {
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
        values.push(DropProbe {
            id: 0,
            counts: std::sync::Arc::new(vec![AtomicUsize::new(0)]),
            panic_at: None,
        });
        drop(values);
        let (values, counts) = probes(Some(35));
        assert!(panics(|| drop(values)));
        for count in counts.iter() {
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn zst_rebuild_uses_finite_exact_slot_counts() {
        let mut values = AppendVec::<()>::with_capacity(2);
        for _ in 0..100 {
            values.push(());
        }
        values.make_contiguous(3);
        assert_eq!(values.contiguous_slots(), 103);
        assert_eq!(values.allocated_slots(), 103);
        values.retain_range_and_compact(90..100, 2);
        assert_eq!(values.len(), 10);
        assert_eq!(values.allocated_slots(), 12);
        values.retain_range_and_compact(10..10, 4);
        assert_eq!(values.len(), 0);
        assert_eq!(values.allocated_slots(), 4);
        values.clear(0);
        assert_eq!(values.allocated_slots(), 0);
    }

    #[test]
    fn concurrent_writers_publish_dense_prefix_and_preserve_old_references() {
        let values = AppendVec::with_capacity(1);
        values.push(String::from("anchor"));
        let anchor = values.get(0).unwrap();
        let address = anchor as *const String;
        let start = std::sync::Barrier::new(5);
        std::thread::scope(|scope| {
            let mut writers = Vec::new();
            for writer in 0..4 {
                let values = &values;
                let start = &start;
                writers.push(scope.spawn(move || {
                    start.wait();
                    let mut indices = Vec::new();
                    for n in 0..300 {
                        let text = format!("{writer}:{n}");
                        let index = values.push(text.clone());
                        assert_eq!(values.get(index), Some(&text));
                        indices.push(index);
                    }
                    indices
                }));
            }
            start.wait();
            while values.len() < 1201 {
                let len = values.len();
                for index in 0..len {
                    assert!(!values.get(index).unwrap().is_empty());
                }
                assert!(values.allocated_slots() >= 1);
                assert_eq!(anchor, "anchor");
                std::thread::yield_now();
            }
            let mut indices = writers
                .into_iter()
                .flat_map(|writer| writer.join().unwrap())
                .collect::<Vec<_>>();
            indices.sort_unstable();
            assert_eq!(indices, (1..1201).collect::<Vec<_>>());
        });
        assert_eq!(values.get(0).unwrap() as *const String, address);
        assert_eq!(values.iter().count(), 1201);
    }

    #[test]
    fn sparse_bucket_statistics_and_integer_boundaries() {
        let values = AppendVec::<()>::with_capacity(3);
        // Leave holes deliberately in spare storage; no T is published or owned here.
        values.storage.load_alloc_ptr(3 + 96);
        assert_eq!(values.allocated_slots(), 131);
        let last = pi_buckets::BUCKETS - 1;
        assert_eq!(
            pi_buckets::Location::bucket_capacity(last),
            pi_buckets::MAX_ENTRIES.checked_add(1).unwrap()
        );
        assert_eq!(
            pi_buckets::Location::of(pi_buckets::MAX_ENTRIES).bucket_index(),
            last
        );
        // Independently worked 32/64-bit arithmetic, without allocating enormous buckets.
        for bits in [32, 64] {
            let max = (1u128 << bits) - 1;
            let tail_end = 4_294_967_264u128;
            assert_eq!(tail_end + 31 <= max, true);
            assert_eq!(tail_end + 32 <= max, bits == 64);
            let prefix = max - 31;
            assert!(prefix + 1 <= max);
            assert!(prefix + 32 > max);
        }
    }

    #[test]
    fn concurrent_noncopy_values_are_dropped_exactly_once() {
        let counts = std::sync::Arc::new((0..400).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let values = AppendVec::default();
        std::thread::scope(|scope| {
            for worker in 0..4 {
                let counts = &counts;
                let values = &values;
                scope.spawn(move || {
                    for id in worker * 100..(worker + 1) * 100 {
                        let index = values.push(DropProbe {
                            id,
                            counts: counts.clone(),
                            panic_at: None,
                        });
                        assert_eq!(values.get(index).unwrap().id, id);
                    }
                });
            }
        });
        assert_eq!(values.len(), 400);
        drop(values);
        for count in counts.iter() {
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn double_destructor_panic_aborts_in_child_process() {
        const CHILD: &str = "PI_APPEND_VEC_DOUBLE_DROP_CHILD";
        if std::env::var_os(CHILD).is_some() {
            struct AlwaysPanics;
            impl Drop for AlwaysPanics {
                fn drop(&mut self) {
                    panic!("double destructor sentinel");
                }
            }
            let values = AppendVec::default();
            values.push(AlwaysPanics);
            values.push(AlwaysPanics);
            drop(values);
            unreachable!();
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "append_vec::append_vec_inner::tests::double_destructor_panic_aborts_in_child_process", "--nocapture"])
            .env(CHILD,"1").env("RUST_BACKTRACE","0").output().unwrap();
        assert!(!output.status.success());
        // A normal test panic exits with 101; nested destructor panic must abort instead.
        assert_ne!(output.status.code(), Some(101));
        assert!(String::from_utf8_lossy(&output.stderr).contains("double destructor sentinel"));
    }

    #[test]
    fn spare_slot_needs_no_default_value() {
        struct NoDefault;
        let _slot = Slot::<NoDefault>::default();
    }
}
impl<'a, T> SharedIter<'a, T> {
    pub(super) fn next_inner(&mut self) -> Option<&'a T> {
        self.slots.next().map(|slot| {
            // SAFETY: the cursor was restricted to the published initialized range.
            unsafe { (&*slot.value.get()).assume_init_ref() }
        })
    }
}
impl<'a, T> ExclusiveIter<'a, T> {
    pub(super) fn next_inner(&mut self) -> Option<&'a mut T> {
        self.slots.next().map(|(_, slot)| {
            // SAFETY: exclusive cursor yields each initialized slot at most once.
            unsafe { slot.value.get_mut().assume_init_mut() }
        })
    }
}
#[cfg(test)]
std::thread_local! {
    static FAILURE: std::cell::Cell<Option<FailurePoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn with_failure_inner<R>(point: FailurePoint, operation: impl FnOnce() -> R) -> R {
    struct Restore(Option<FailurePoint>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FAILURE.with(|configuration| configuration.set(self.0));
        }
    }
    let _restore = Restore(FAILURE.with(|configuration| configuration.replace(Some(point))));
    operation()
}
#[cfg(test)]
pub(super) fn fail_if_requested_inner(point: FailurePoint) {
    if FAILURE.with(|configuration| configuration.get() == Some(point)) {
        panic!("injected preparation failure");
    }
}
impl<'a, T> RemovedValues<'a, T> {
    pub(super) unsafe fn new_inner(
        storage: &'a mut SegmentedArr<Slot<T>>,
        pending: [Range<usize>; 2],
    ) -> Self {
        Self { storage, pending }
    }
    pub(super) fn drain_inner(&mut self) {
        for pending in &mut self.pending {
            while pending.start < pending.end {
                // SAFETY: the guard owns exactly the initialized pending ranges.
                let value = unsafe { self.storage.get_unchecked(pending.start) }
                    .value
                    .get();
                pending.start += 1;
                // Revoke ownership before user Drop; unwinding resumes at the next slot.
                unsafe { (*value).assume_init_drop() };
            }
        }
    }
    /// 复用 RemovedValues::drop 的接续契约；不新增回收流程。
    pub(super) fn drop_inner(&mut self) {
        self.drain()
    }
}
impl<T> Slot<T> {
    pub(super) fn default_inner() -> Self {
        Self {
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}
impl<T> AppendVec<T> {
    /// 复用 AppendVec::default 的零槽位构造契约。
    pub(super) fn default_inner() -> Self {
        Self::with_capacity(0)
    }
    pub(super) fn with_capacity_inner(contiguous_slots: usize) -> Self {
        Self {
            commit_lock: Mutex::new(()),
            storage: SegmentedArr::with_capacity(contiguous_slots),
            len: AtomicUsize::new(0),
        }
    }
    pub(super) fn len_inner(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
    pub(super) fn is_empty_inner(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn push_inner(&self, value: T) -> usize {
        // The argument outlives the guard: preparation unwind unlocks before dropping value.
        let guard = self.commit_lock.lock().unwrap_or_else(|poisoned| {
            let guard = poisoned.into_inner();
            self.commit_lock.clear_poison();
            guard
        });
        let index = self.len.load(Ordering::Acquire);
        let next_len = index.checked_add(1).expect("length overflow");
        let contiguous = self.storage.contiguous_len();
        if index >= contiguous {
            let location = pi_buckets::Location::of(index - contiguous);
            contiguous
                .checked_add(pi_buckets::Location::bucket_capacity(
                    location.bucket_index(),
                ))
                .expect("capacity overflow");
        }
        #[cfg(test)]
        fail_if_requested(FailurePoint::AppendBeforeAllocation);
        let slot = self.storage.load_alloc_ptr(index);
        #[cfg(test)]
        fail_if_requested(FailurePoint::AppendAfterPreparation);
        // SAFETY: the lock uniquely owns this unpublished slot. Never create &mut Slot.
        // Preparation is complete; writing and Release publication cannot unwind.
        unsafe { (*slot).value.get().write(MaybeUninit::new(value)) };
        self.len.store(next_len, Ordering::Release);
        drop(guard);
        index
    }
    pub(super) fn get_inner(&self, index: usize) -> Option<&T> {
        if index < self.len() {
            // SAFETY: Acquire observed the initialized prefix; shared self excludes moves.
            Some(unsafe { self.get_unchecked(index) })
        } else {
            None
        }
    }
    pub(super) unsafe fn get_unchecked_inner(&self, index: usize) -> &T {
        // SAFETY: caller has observed publication and excludes conflicting mutation.
        unsafe { (&*self.storage.get_unchecked(index).value.get()).assume_init_ref() }
    }
    pub(super) fn get_mut_inner(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len() {
            // SAFETY: the checked index is initialized and self is exclusively borrowed.
            Some(unsafe { self.get_unchecked_mut(index) })
        } else {
            None
        }
    }
    pub(super) unsafe fn get_unchecked_mut_inner(&mut self, index: usize) -> &mut T {
        // SAFETY: caller guarantees initialization and the exclusive self borrow excludes aliases.
        unsafe {
            self.storage
                .get_unchecked_mut(index)
                .value
                .get_mut()
                .assume_init_mut()
        }
    }
    #[allow(clippy::mut_from_ref)] // The public unsafe contract requires target exclusivity.
    pub(super) unsafe fn load_inner(&self, index: usize) -> Option<&mut T> {
        if index < self.len() {
            // SAFETY: Acquire proves initialization; caller guarantees target exclusivity.
            Some(unsafe { self.load_unchecked(index) })
        } else {
            None
        }
    }
    #[allow(clippy::mut_from_ref)] // The public unsafe contract requires target exclusivity.
    pub(super) unsafe fn load_unchecked_inner(&self, index: usize) -> &mut T {
        // SAFETY: caller observed publication and excludes all conflicting target accesses.
        // Only UnsafeCell's content is mutably borrowed, never the enclosing Slot/storage.
        unsafe { (&mut *self.storage.get_unchecked(index).value.get()).assume_init_mut() }
    }
    pub(super) fn iter_inner(&self) -> SharedIter<'_, T> {
        let len = self.len();
        SharedIter {
            slots: self.storage.slice(0..len),
        }
    }
    pub(super) fn iter_mut_inner(&mut self) -> ExclusiveIter<'_, T> {
        let len = self.len();
        ExclusiveIter {
            slots: self.storage.slice_mut(0..len),
        }
    }
    pub(super) fn slice_inner(&self, range: Range<usize>) -> SharedIter<'_, T> {
        let len = self.len();
        assert!(
            range.start <= range.end && range.end <= len,
            "invalid slice range"
        );
        SharedIter {
            slots: self.storage.slice(range),
        }
    }
    pub(super) fn contiguous_slots_inner(&self) -> usize {
        self.storage.contiguous_len()
    }
    pub(super) fn allocated_slots_inner(&self) -> usize {
        let contiguous = self.storage.contiguous_len();
        let mut total = contiguous;
        for bucket in 0..pi_buckets::BUCKETS {
            let start =
                contiguous.checked_add(pi_buckets::Location::bucket_len(bucket) - pi_buckets::SKIP);
            let end = contiguous.checked_add(pi_buckets::Location::bucket_capacity(bucket));
            let (Some(start), Some(end)) = (start, end) else {
                break;
            };
            for segment in self.storage.segments(start..end) {
                total = total
                    .checked_add(segment.len())
                    .expect("allocated slots overflow");
            }
        }
        total
    }
    pub(super) fn make_contiguous_inner(&mut self, additional: usize) {
        self.retain_range_and_compact(0..self.len(), additional);
    }
    pub(super) fn retain_range_and_compact_inner(
        &mut self,
        range: Range<usize>,
        additional: usize,
    ) {
        let old_len = self.len();
        assert!(
            range.start <= range.end && range.end <= old_len,
            "invalid retained range"
        );
        let retained_len = range.end - range.start;
        let target_slots = retained_len
            .checked_add(additional)
            .expect("capacity overflow");
        #[cfg(test)]
        fail_if_requested(FailurePoint::RebuildBeforeAllocation);
        let mut target = SegmentedArr::<Slot<T>>::with_capacity(target_slots);
        // Prepare checked cursors before moving any ownership. Both cover allocated slots.
        let source = self.storage.slice(range.clone());
        let destination = target.slice_mut(0..retained_len);
        #[cfg(test)]
        fail_if_requested(FailurePoint::RebuildAfterPreparation);
        // SAFETY: exclusive self owns all source values; source and destination allocations
        // are disjoint and each cursor visits exactly retained_len initialized Slot objects.
        // No allocation, user callback, or fallible validation occurs during this transfer.
        for (source, (_, destination)) in source.zip(destination) {
            unsafe {
                destination
                    .value
                    .get_mut()
                    .write((*source.value.get()).assume_init_read());
            }
        }
        let mut old_storage = std::mem::replace(&mut self.storage, target);
        *self.len.get_mut() = retained_len;
        // SAFETY: moved values are excluded; these two disjoint ranges retain sole ownership.
        // The owner is bound before the guard, so it survives guard unwinding and cleanup.
        let mut removed =
            unsafe { RemovedValues::new(&mut old_storage, [0..range.start, range.end..old_len]) };
        removed.drain();
    }
    pub(super) fn clear_inner(&mut self, additional: usize) {
        self.retain_range_and_compact(0..0, additional);
    }
}
impl<T> AppendVec<T> {
    pub(super) fn index_inner(&self, index: usize) -> &T {
        self.get(index).expect("index out of bounds")
    }
    pub(super) fn index_mut_inner(&mut self, index: usize) -> &mut T {
        self.get_mut(index).expect("index out of bounds")
    }
    pub(super) fn drop_inner(&mut self) {
        let old_len = *self.len.get_mut();
        *self.len.get_mut() = 0;
        // SAFETY: exclusive self owns the entire initialized prefix. Spare slots own no T.
        let mut removed = unsafe { RemovedValues::new(&mut self.storage, [0..old_len, 0..0]) };
        removed.drain();
    }
}
impl<T: std::fmt::Debug> AppendVec<T> {
    pub(super) fn fmt_inner(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
