//! 一个已发布前缀的追加容器：短锁串行写入，读取不取得提交锁。
#[path = "append_vec_inner.rs"]
mod append_vec_inner;
use pi_arr::SegmentedArr;
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::ops::{Index, IndexMut, Range};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

/// # 用途
/// 追加成功后立即可读的容器；不要求 T: Default。
/// 核心不变量：0..len 恰好是已初始化前缀，后续槽位未持有 T；追加不移动旧值。
/// 存储固定使用 SegmentedArr，不随依赖 rc feature 切换；所有共享写入只能经私有 Slot::value 的 UnsafeCell，不创建整体独占引用。
/// 提交锁的释放/获取传递前序写入，再由 len 的 Release/Acquire 向读者发布完整前缀。
/// Send 要求 T: Send，Sync 要求 T: Send + Sync；单线程使用仍允许不满足这些约束的 T。
/// 不给 Slot 单独声明 Sync；使用明确的分段存储类型，避免连续数组共享增长使旧引用失效。
///
/// 正向只验证类型，不构造或析构设计桩：
/// ```
/// use pi_append_vec::AppendVec;
/// use std::{cell::Cell, rc::Rc};
/// fn send<T: Send>() {}
/// fn sync<T: Sync>() {}
/// send::<AppendVec<Cell<u8>>>();
/// send::<AppendVec<String>>();
/// sync::<AppendVec<String>>();
/// fn single_thread(_: Option<AppendVec<Rc<u8>>>) {}
/// single_thread(None);
/// ```
/// 非 Send 元素不能转移整个容器：
/// ```compile_fail,E0277
/// use pi_append_vec::AppendVec;
/// fn send<T: Send>() {}
/// send::<AppendVec<std::rc::Rc<u8>>>();
/// ```
/// Send 但非 Sync 的元素不能共享容器：
/// ```compile_fail,E0277
/// use pi_append_vec::AppendVec;
/// fn sync<T: Sync>() {}
/// sync::<AppendVec<std::cell::Cell<u8>>>();
/// ```
/// Sync 但非 Send 的元素也不能共享容器（共享 push 会转移元素所有权）：
/// ```compile_fail,E0277
/// use pi_append_vec::AppendVec;
/// struct SyncOnly(std::rc::Rc<()>);
/// unsafe impl Sync for SyncOnly {}
/// fn sync<T: Sync>() {}
/// sync::<AppendVec<SyncOnly>>();
/// ```
pub struct AppendVec<T> {
    // 只串行化追加提交，不保护已发布值的只读生命周期。
    commit_lock: Mutex<()>,
    // 地址稳定的连续区与分段桶；独占整理允许移动，不能整体放进 UnsafeCell。
    storage: SegmentedArr<Slot<T>>,
    // 唯一的逻辑长度；写入完成后 Release 发布，读取 Acquire 获取；声明顺序保持在最后。
    len: AtomicUsize,
}

// SAFETY: storage 私有且唯一拥有槽位；T: Send 允许值及其析构责任随容器转移。
// 移动容器本身不移动堆上槽位；安全借用禁止带着存活元素引用转移所有者。
// 整理、回收仅经 &mut self，不能与共享访问重叠；不为 Slot 增加 Sync。
unsafe impl<T: Send> Send for AppendVec<T> {}
// SAFETY: 安全共享写仅在 commit_lock 内经 Slot::value.get() 写未发布尾槽位，
// 不产生 &mut Slot、&mut SegmentedArr 或覆盖既有值的独占引用。底层原子桶发布
// 保护 Slot 初始化；提交锁交接传递所有先前写入，len Release/Acquire 发布整个前缀。
// T: Send 覆盖跨线程 push 的所有权转移，T: Sync 覆盖 get/迭代返回的共享引用。
// unsafe load 由调用方排除目标的全部冲突借用并同步；它不是安全并发写入口。
// &mut self 独占搬移与析构排除读者；实现必须维持上述边界，不能依赖底层 Slot: Sync。
unsafe impl<T: Send + Sync> Sync for AppendVec<T> {}

/// # 用途
/// 私有共享迭代表示；底层游标只覆盖一次已发布快照或严格校验的子范围。
struct SharedIter<'a, T> {
    // 分段 Slot 共享游标；不把 Slot 切片强转成 T 切片，每次只投影一个已初始化 T。
    slots: pi_buckets::BucketIter<'a, Slot<T>>,
}

impl<'a, T> Iterator for SharedIter<'a, T> {
    type Item = &'a T;
    /// # 用途
    /// 返回下个已初始化 T 的共享引用，耗尽返回 None；不分配，不读取快照之外的槽位。
    fn next(&mut self) -> Option<Self::Item> {
        self.next_inner()
    }
}

/// # 用途
/// 私有独占迭代表示；由 &mut AppendVec 的已初始化前缀建立，元素至多返回一次。
struct ExclusiveIter<'a, T> {
    // 底层分段游标返回索引和独占 Slot；丢弃索引并逐槽投影，不跨分配构造切片。
    slots: pi_arr::SegmentedIterMut<'a, Slot<T>>,
}

impl<'a, T> Iterator for ExclusiveIter<'a, T> {
    type Item = &'a mut T;
    /// # 用途
    /// 返回下个互不重叠的独占 T 引用，耗尽返回 None；仅访问已初始化前缀，不分配。
    fn next(&mut self) -> Option<Self::Item> {
        self.next_inner()
    }
}

/// # 用途
/// 单个物理槽位；是否持有 T 完全由容器已发布长度及独占搬移所有权决定。
struct Slot<T> {
    // 安全追加仅写尾部未发布槽位；发布后共享读，unsafe load 仅在调用方保证目标独占时可修改。
    value: UnsafeCell<MaybeUninit<T>>,
}

/// # 用途
/// 测试专用准备阶段故障点；实现时仅在当前线程生效，不改变生产结构或公开 API。
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FailurePoint {
    // 追加持有提交锁、验证通过，但尚未分配槽位；验证无新增容量的准备失败。
    AppendBeforeAllocation,
    // 追加已准备槽位，尚未写入 T；验证备用容量可增长、锁中毒及 value 锁外析构。
    AppendAfterPreparation,
    // 重建已校验范围和容量、尚未分配目标；旧容器必须保持原状。
    RebuildBeforeAllocation,
    // 新存储和回收资源已准备，尚未移动任何 T；旧容器仍须保持原状。
    RebuildAfterPreparation,
}

/// # 用途
/// 参数 point 为本线程目标点，operation 为经公开壳入口执行的验收闭包；返回闭包结果。
/// 命中时以 panic 展开；无论正常返回或展开，RAII 恢复前一个线程局部配置，允许嵌套。
/// 不持有全局锁执行闭包，不影响其他线程；当前仅为接线声明桩，验收暂不运行。
#[cfg(test)]
/// # 用途
/// # Examples
/// 测试模块在已确认的准备阶段注入失败；以下为同模块内的接线示意。
/// ```no_run
/// use pi_append_vec::AppendVec;
/// // 仅供内部测试模块使用，运行时将进入当前设计桩。
/// let values = AppendVec::with_capacity(0);
/// // 内部测试以 with_failure 包裹此公开入口，检查准备失败后的状态。
/// let _operation = || values.push(7);
/// ```
fn with_failure<R>(point: FailurePoint, operation: impl FnOnce() -> R) -> R {
    append_vec_inner::with_failure_inner(point, operation)
}

/// # 用途
/// 参数 point 为准备流程到达的检查点；仅命中本线程配置时 panic，否则无副作用。
/// 由 push 及三个重建入口的共同准备流程在壳指定位置调用；所有权提交段禁止调用。
/// 当前是设计接线声明，具体调用随业务桩实现；生产构建不含此接口。
#[cfg(test)]
/// # 用途
/// # Examples
/// 检查点由内部准备流程调用；公开入口无需额外配置。
/// ```no_run
/// use pi_append_vec::AppendVec;
/// let values = AppendVec::with_capacity(0);
/// // 测试构建在 push 的准备阶段调用检查点，所有权提交段禁止注入。
/// let _index = values.push(7);
/// ```
fn fail_if_requested(point: FailurePoint) {
    append_vec_inner::fail_if_requested_inner(point)
}

/// # 用途
/// 独占回收旧存储中两个互不重叠的初始化区间，不拥有存储分配本身。
/// 不变量：只有 pending 中尚未推进的索引仍归本守卫析构；已搬走的保留值不在区间内。
/// 存储所有者必须活到守卫析构之后；守卫本身及其 drain 均不得分配。
struct RemovedValues<'a, T> {
    // 旧存储的独占借用，防止值回收完成前释放存储或产生冲突访问。
    storage: &'a mut SegmentedArr<Slot<T>>,
    // 尚未析构的两段原索引范围；range.start 为各段下一个待处理索引，空范围表示完成。
    pending: [Range<usize>; 2],
}

impl<'a, T> RemovedValues<'a, T> {
    /// # 用途
    /// 参数 storage 为存活的旧存储，pending 为待回收的两段索引范围；无分配地建立析构责任。
    /// # Safety
    /// pending 必须合法、互不重叠，每个索引均已分配且恰好持有一个 T；不存在其他析构责任或借用。
    /// 已转移到新存储的值不得包含在 pending 内；storage 的所有者必须在返回守卫之后析构。
    /// 本入口只转移析构责任，不调用用户代码；当前为显式未实现桩。
    unsafe fn new(storage: &'a mut SegmentedArr<Slot<T>>, pending: [Range<usize>; 2]) -> Self {
        unsafe { Self::new_inner(storage, pending) }
    }

    /// # 用途
    /// 析构当前尚待回收的所有值；不分配，耗尽后再次调用无副作用。
    /// 每次先取得已分配槽位地址并推进 pending，再启动 T::drop；用户 panic 原样传播。
    /// 正常路径须显式调用本方法，使首个用户 panic 展开时由 Self::drop 接续回收。
    /// # Examples
    /// 内部 drain 服务于移除值回收；外部由 clear 等入口触发。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(1);
    /// values.push(String::from("待回收"));
    /// values.clear(0);
    /// ```
    fn drain(&mut self) {
        // 设计已确认：值回收游标的唯一流程主体。
        // 取第一个非空区间 → 定位有效槽位 → 先推进区间起点 → 调用当前 T 的析构
        // 全部为空则返回；首个用户 panic 交栈展开处理，不回退已推进游标。
        self.drain_inner()
    }
}

impl<T> Drop for RemovedValues<'_, T> {
    /// # 用途
    /// 接续 drain 的剩余工作，不重复启动任何已推进值的析构；不分配。
    /// 若原已在展开，剩余值再次 panic 将按 Rust 规则终止；不吞掉用户 panic。
    /// # Examples
    /// 通过所有者离开作用域触发析构；内部守卫按既定规则接续回收。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(1);
    /// values.push(String::from("待回收"));
    /// drop(values);
    /// ```
    fn drop(&mut self) {
        self.drop_inner()
    }
}

impl<T> Default for Slot<T> {
    /// # 用途
    /// 返回有效但不持有 T 的备用槽位；不分配、不调用 T 的 Default 或 Drop，不要求 T: Default。
    /// 当前为未实现声明桩；实现须仅包装 UnsafeCell::new(MaybeUninit::uninit())，不能引入用户回调。
    fn default() -> Self {
        Self::default_inner()
    }
}

impl<T> AppendVec<T> {
    /// # 用途
    /// 参数 contiguous_slots 为初始连续槽位数；返回已发布长度为零的容器，不构造 T。分配或容量溢出可 panic。
    pub fn with_capacity(contiguous_slots: usize) -> Self {
        Self::with_capacity_inner(contiguous_slots)
    }
    /// # 用途
    /// Acquire 读取已发布长度快照；0..len 全部已初始化，无预留空洞；不加提交锁、不分配。
    pub fn len(&self) -> usize {
        self.len_inner()
    }
    /// # 用途
    /// 返回已发布长度是否为零；与 len 使用相同观察语义，不加提交锁。
    pub fn is_empty(&self) -> bool {
        self.is_empty_inner()
    }
    /// # 用途
    /// 参数 value 为转移给容器的新值；返回提交顺序确定的索引，返回时已经发布。
    /// 写入者通过短锁串行提交；旧元素引用与并发追加可共存。分配/溢出失败不得增长长度。
    /// 允许在锁内扩容；可能等待提交锁和分配，不承诺固定耗时，不是无锁写入。
    /// 容量/索引溢出或可展开分配失败使用 panic；失败时旧值和已发布长度不变，备用槽位可增长。
    /// 提交锁因准备阶段展开而中毒时，下次追加基于完整前缀不变量恢复并清除中毒，不要求 clear。
    /// value 尚未写入时仍由本次调用拥有，展开路径先释放提交锁再析构 value；OOM abort 不作恢复保证。
    /// 当前实现为显式未实现桩；上述是已确认契约，不是运行验证结论。
    /// # Examples
    /// 追加返回的索引已经发布。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(0);
    /// let index = values.push(7);
    /// assert_eq!(values.get(index), Some(&7));
    /// ```
    pub fn push(&self, value: T) -> usize {
        // 设计已确认：追加与失败恢复的唯一主体。
        // value 由调用栈持有 → 获取锁；准备阶段中毒则恢复 guard 并清除中毒
        // 读当前已发布长度作为索引 → 检查索引/长度溢出 → 准备地址稳定的尾槽位
        // 尾槽位落入桶时，以 Location::of(index - contiguous_slots) 验证桶边界；
        // 在分配前 checked_add(contiguous_slots, Location::bucket_capacity(bucket))，
        // 拒绝整桶物理槽位总数不可表示的容量；不能只检查 index + 1。
        // 测试构建在分配前与槽位准备后分别接入 FailurePoint 的两个追加注入点。
        // 槽位准备使用 load_alloc_ptr 或等效稳定地址接口；写入仅经 Slot::value.get()，不借出 &mut Slot
        // 任一步展开：先释放 guard，再析构尚未移入的 value；旧前缀不变
        // 移入 value → Release 发布新长度；这两步之间不得有可展开操作
        // 释放 guard → 返回索引；不在锁内执行用户 T 的析构或回调。
        self.push_inner(value)
    }
    /// # 用途
    /// 参数 index 为从零开始的逻辑索引；先 Acquire 观察长度，越界返回 None，否则返回已初始化共享引用。
    /// 不加提交锁、不分配；借用期间追加不移动旧值，整理和清理由独占借用排除。
    pub fn get(&self, index: usize) -> Option<&T> {
        self.get_inner(index)
    }
    /// # 用途
    /// 参数 index 为逻辑索引；返回共享引用，不分配。
    /// # Safety
    /// 调用方保证 index 小于已经 Acquire 观察到的已发布长度，且无冲突可变访问。
    /// # Examples
    /// 先同步观察长度，并排除目标的冲突可变访问。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// assert!(index < values.len());
    /// // 安全：已观察到发布，目标没有可变引用或并发修改。
    /// assert_eq!(unsafe { values.get_unchecked(index) }, &7);
    /// ```
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        unsafe { self.get_unchecked_inner(index) }
    }
    /// # 用途
    /// 参数 index 为逻辑索引；越界返回 None，否则返回独占引用。引用期间独占容器，不分配。
    /// # Examples
    /// 借出元素时独占容器。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// *values.get_mut(index).unwrap() = 8;
    /// ```
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.get_mut_inner(index)
    }
    /// # 用途
    /// 参数 index 为逻辑索引；返回独占引用，不分配。
    /// # Safety
    /// index 必须小于已发布长度；维持 self 独占借用，禁止冲突裸指针访问。
    /// # Examples
    /// 独占容器并确认索引已发布。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// // 安全：索引已发布，容器独占且没有冲突裸指针访问。
    /// *unsafe { values.get_unchecked_mut(index) } = 8;
    /// ```
    pub unsafe fn get_unchecked_mut(&mut self, index: usize) -> &mut T {
        unsafe { self.get_unchecked_mut_inner(index) }
    }
    /// # 用途
    /// 参数 index 为逻辑索引；Acquire 观察已发布长度，越界返回 None，否则返回目标元素的独占引用。
    /// 不分配、不取得提交锁、不改变长度；命名对应 SegmentedArr::load，而非 get_mut。
    /// # Safety
    /// 返回引用存活期间，目标 T 不得存在其他共享或可变引用及任何冲突并发访问，包括 get、索引及迭代所得引用。
    /// 调用方负责跨线程同步；存储必须存活，不得与整理、移除或释放重叠。
    /// 对其他槽位的追加可共存，但提交锁不保护本引用；必须仅经 Slot::value 访问目标 T，不能构造整体 &mut Slot。
    /// 当前实现为显式未实现桩。
    /// # Examples
    /// 调用方排除目标的全部其他引用和并发访问。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// // 安全：单线程，目标没有其他引用；修改结束前不读取、迭代或整理。
    /// *unsafe { values.load(index) }.unwrap() = 8;
    /// ```
    pub unsafe fn load(&self, index: usize) -> Option<&mut T> {
        unsafe { self.load_inner(index) }
    }

    /// # 用途
    /// 参数 index 为已经发布的逻辑索引；直接返回独占元素引用，不分配、不取得提交锁。
    /// # Safety
    /// 调用方必须已通过 Acquire 或等效同步观察到 index < len；并满足 Self::load 的全部独占、生命周期与同步约束。
    /// 不承担安全边界检查；无效索引违反前提，不能依赖 panic 检测。当前实现为显式未实现桩。
    /// # Examples
    /// 同时保证已发布边界与目标排他访问。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// assert!(index < values.len());
    /// // 安全：已同步观察发布，单线程且目标没有其他引用，存储保持存活。
    /// *unsafe { values.load_unchecked(index) } = 8;
    /// ```
    pub unsafe fn load_unchecked(&self, index: usize) -> &mut T {
        unsafe { self.load_unchecked_inner(index) }
    }

    /// # 用途
    /// 取得一次已发布长度快照，只遍历该前缀；返回共享引用，不含随后追加的值，不加提交锁。
    /// # Examples
    /// 共享迭代只覆盖建立时的前缀。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(0);
    /// values.push(7);
    /// let snapshot = values.iter();
    /// values.push(8);
    /// assert_eq!(snapshot.copied().collect::<Vec<_>>(), vec![7]);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.iter_inner()
    }
    /// # 用途
    /// 独占遍历已发布前缀，返回互不重叠的可变引用；迭代期间独占容器，不分配。
    /// # Examples
    /// 迭代期间独占容器。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(0);
    /// values.push(7);
    /// for value in values.iter_mut() {
    ///     *value += 1;
    /// }
    /// ```
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.iter_mut_inner()
    }
    /// # 用途
    /// 参数 range 为逻辑半开范围；以一次 Acquire 长度快照检查 start <= end <= len，只遍历该范围，不分配。
    /// 允许 0..0 和 len..len 等合法空范围，不包含随后追加的值；不加提交锁。
    /// # Panics
    /// 反向或超出该长度快照时，在读取元素前 panic，不裁剪范围、不改变容器。
    /// 当前实现为显式未实现桩。
    /// # Examples
    /// 范围必须完整落在已发布前缀内。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(0);
    /// values.push(7);
    /// assert_eq!(values.slice(0..1).copied().collect::<Vec<_>>(), vec![7]);
    /// assert_eq!(values.slice(1..1).count(), 0);
    /// ```
    pub fn slice(&self, range: Range<usize>) -> impl Iterator<Item = &T> {
        self.slice_inner(range)
    }
    /// # 用途
    /// 返回连续存储区可访问的物理槽位数，包含未写入的备用槽位，不等于已发布长度或分配器保留字节。
    /// 零大小类型同样返回有限槽位数；不加提交锁、不分配。
    pub fn contiguous_slots(&self) -> usize {
        self.contiguous_slots_inner()
    }
    /// # 用途
    /// 返回连续区与已分配桶的槽位总数近似观察值，包含未写入槽位；不取得提交锁、不分配。
    /// 并发追加时可能漏计统计期间新分配的桶，不保证与单独调用的 len 构成一致快照。
    /// 无并发写入且操作完成后应返回准确总数；不是分配器保留字节，也不能用作元素访问或免分配保证。
    /// 枚举界限来自 pi_buckets 公开常量和 Location，不猜测桶数；精确规则见方法流程。
    /// 容量准备保证完整物理槽位总数可由 usize 表示；当前为显式未实现桩。
    pub fn allocated_slots(&self) -> usize {
        // 设计已确认：先计 contiguous_len，再遍历 bucket in 0..pi_buckets::BUCKETS。
        // 桶相对起点 = Location::bucket_len(bucket) - SKIP，尾后 = Location::bucket_capacity(bucket)。
        // 绝对起止均 checked_add 连续长度；若任一溢出则停止（容量准备禁止分配该桶及后续桶）。
        // 对每个完整绝对范围调用 storage.segments(range)，累计返回切片 len；未分配桶被跳过。
        // 起止均在同一桶内，不构造跨分配切片；每桶一次 Acquire 观察，可漏计稍后发布的桶。
        // checked_add 累计，按容量不变量不可能失败；不得饱和、截断或以近似掩盖计数溢出。
        // MAX_ENTRIES 是相对最大索引，半开尾后为 checked_add(MAX_ENTRIES, 1)；
        // 不能先算连续长度 + 全部桶最大容量：32 位下可能溢出，即使实际只分配少量桶。
        self.allocated_slots_inner()
    }
    /// # 用途
    /// 参数 additional 为重建后的备用连续槽位数；结果连续槽位数精确为 len + additional，不保留旧备用容量。
    /// 独占搬移全部值，保持长度和顺序，旧地址失效；加法溢出在修改前 panic。
    /// 先完成目标存储及转移所需资源的准备；容量溢出或可展开分配失败时旧值、顺序和长度保持不变。
    /// 准备成功后，无可展开操作地转移值并安装新存储及长度；不析构被转移的 T，不构造默认 T。
    /// 当前实现为显式未实现桩；不保证复用原分配或物理容量保持不变。
    /// # Examples
    /// 独占重建，并精确保留指定备用槽位。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(0);
    /// values.push(7);
    /// values.make_contiguous(2);
    /// assert_eq!(values.contiguous_slots(), 3);
    /// ```
    pub fn make_contiguous(&mut self, additional: usize) {
        // 设计已确认：采用 Self::retain_range_and_compact 的准备与所有权提交顺序。
        // 差异：保留范围为全部已发布值，不产生待析构的 T，长度不变。
        self.make_contiguous_inner(additional)
    }
    /// # 用途
    /// 参数 range 为保留范围，additional 为重建后的备用连续槽位数；保留值前移到零并更新长度，其余值恰好析构一次。
    /// 结果连续槽位数精确为 range.len() + additional，不保留旧备用容量或分段桶；加法溢出在修改前 panic。
    /// 先完成范围/容量验证及目标存储、回收守卫所需资源的准备；准备失败时旧值与长度不变。
    /// 转移后先安装保留值和新长度，随后才析构被移除值；每个值的析构最多启动一次。
    /// 用户 Drop panic 时传播展开，容器已处于保留结果状态，可继续使用和析构；守卫负责清理其余移除值。
    /// 展开时再次发生 Drop panic 遵循 Rust 的进程终止语义，不保证从双重 panic 或 OOM abort 恢复。
    /// # Panics
    /// range 必须满足 start <= end <= 当前长度；非法范围在分配、移动或析构前 panic，容器不变，不裁剪。
    /// 合法空范围移除全部值；容量溢出或可展开分配失败同样保持容器不变。
    /// 当前实现为显式未实现桩。
    /// # Examples
    /// 保留合法范围并重建为精确目标槽位数。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(0);
    /// values.push(7);
    /// values.push(8);
    /// values.retain_range_and_compact(1..2, 2);
    /// assert_eq!(values.get(0), Some(&8));
    /// assert_eq!(values.contiguous_slots(), 3);
    /// ```
    pub fn retain_range_and_compact(&mut self, range: Range<usize>, additional: usize) {
        // 设计已确认：独占重建与移除值析构的唯一流程主体。
        // 验证范围与目标槽位数 → 准备新存储和回收守卫的全部资源；失败不改变 self
        // 测试构建在目标分配前、准备完成后进入所有权提交前接入重建失败点；提交段不注入。
        // 无可展开操作地转移保留值，移除值所有权交给局部守卫；旧槽位不再拥有被移动值
        // 安装新存储和新长度，使 self 先成为有效保留结果
        // 旧所有者与 RemovedValues 按析构顺序绑定；守卫区间为 0..range.start 和 range.end..old_len
        // 显式调用 RemovedValues::drain；首个 panic 展开时由守卫 Drop 接续，旧存储最后释放
        // 始终避免重复析构；展开中再次 panic 按 Rust 规则终止。
        self.retain_range_and_compact_inner(range, additional)
    }
    /// # 用途
    /// 参数 additional 为清理后精确的备用连续槽位数；移除全部已发布值并将长度归零，之后可再次追加。
    /// 不保留旧备用容量或分段桶；clear(0) 安装无槽位空存储并回收旧存储，空容器也遵循本规则。
    /// 提交锁准备阶段中毒由后续 push 自动恢复，clear 不承担该恢复前置职责。
    /// 准备失败时容器不变；准备成功后先安装空的新存储并将长度置零，再析构旧值。
    /// 用户 Drop panic 时容器已为空且可继续使用，旧值按 Self::retain_range_and_compact 的守卫规则回收。
    /// 当前实现为显式未实现桩。
    /// # Examples
    /// 清空后仍可追加；零备用槽位回收全部旧存储。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(1);
    /// values.push(7);
    /// values.clear(0);
    /// assert!(values.is_empty());
    /// assert_eq!(values.allocated_slots(), 0);
    /// assert_eq!(values.push(8), 0);
    /// ```
    pub fn clear(&mut self, additional: usize) {
        // 设计已确认：采用 Self::retain_range_and_compact 的准备、安装、析构顺序。
        // 差异：保留范围为空，先准备精确 additional 个备用槽位，安装后的长度为零。
        self.clear_inner(additional)
    }
}
impl<T> Default for AppendVec<T> {
    /// # 用途
    /// 返回长度为零且无初始连续槽位的容器。
    fn default() -> Self {
        Self::default_inner()
    }
}
impl<T> Index<usize> for AppendVec<T> {
    type Output = T;
    /// # 用途
    /// 参数 index 为逻辑索引；越界 panic，否则借出共享引用，不加提交锁。
    /// # Examples
    /// 索引必须位于已发布前缀。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// assert_eq!(values[index], 7);
    /// ```
    fn index(&self, index: usize) -> &T {
        self.index_inner(index)
    }
}
impl<T> IndexMut<usize> for AppendVec<T> {
    /// # 用途
    /// 参数 index 为逻辑索引；越界 panic，否则借出独占引用。
    /// # Examples
    /// 通过独占容器修改已发布值。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let mut values = AppendVec::with_capacity(1);
    /// let index = values.push(7);
    /// values[index] = 8;
    /// ```
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.index_mut_inner(index)
    }
}
impl<T: std::fmt::Debug> std::fmt::Debug for AppendVec<T> {
    /// # 用途
    /// 参数 f 为格式输出目标；共享遍历快照，格式输出失败返回错误。
    /// # Examples
    /// 格式化只共享读取发布快照。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(0);
    /// values.push(7);
    /// assert_eq!(format!("{values:?}"), "[7]");
    /// ```
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_inner(f)
    }
}
impl<T> Drop for AppendVec<T> {
    /// # 用途
    /// 仅析构已初始化值，未写入备用槽位不得按 T 析构。
    /// 不分配新存储；先将长度归零，把旧前缀 0..old_len 与一个空范围交给 RemovedValues。
    /// 保持 storage 字段存活，显式 drain；用户 panic 时局部守卫先回收剩余值，字段存储随后释放。
    /// 逐值析构的所有权撤销顺序同 Self::retain_range_and_compact，用户 Drop panic 时守卫清理剩余值。
    /// 展开中再次 panic 按 Rust 规则终止；当前实现为显式未实现桩。
    /// # Examples
    /// 通过所有者离开作用域触发析构；内部守卫按既定规则接续回收。
    /// ```no_run
    /// use pi_append_vec::AppendVec;
    /// let values = AppendVec::with_capacity(1);
    /// values.push(String::from("待回收"));
    /// drop(values);
    /// ```
    fn drop(&mut self) {
        self.drop_inner()
    }
}
