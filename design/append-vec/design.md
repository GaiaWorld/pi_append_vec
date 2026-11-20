# AppendVec 设计

实现根：src/。壳：src/append_vec.rs；芯：src/append_vec_inner.rs；crate 入口：src/lib.rs。
设计轻审与复杂实现流程均已完成；当前验证命令为 cargo check --all-targets、cargo test、cargo fmt -- --check。实际最终结果见“当前实现验收”。

## 已确认需求与取舍

用户确认只保留 AppendVec 一个容器，采用短锁串行提交，不提前取得索引。移除 PublishedVec、Entry、reserve_entry 以及原始预留层。push(&self, value) 返回提交顺序确定的索引，返回时该值已经发布。len 只表示已发布长度，不再维护预留计数。

共享读取不取得提交锁，追加不移动已有值；安全可变访问、连续化、保留范围和清理要求 &mut self；unsafe load 系列允许通过 &self 在调用方保证目标独占时借出 &mut T。调用者在容器外准备 T，取消准备不影响容器。容器不要求 T: Default。

接口名称已落实：push、make_contiguous、retain_range_and_compact。contiguous_slots 表示连续区物理槽位数，allocated_slots 表示全部已分配物理槽位数，len 表示有效值数量。

## 核心结构与路径

源码真实定义 AppendVec<T> 的提交锁 commit_lock、分段存储 storage、唯一已发布长度 len；用户要求 len 声明在字段最后，此顺序不构成 Rust 默认表示的物理布局承诺；Slot<T> 用 UnsafeCell<MaybeUninit<T>> 表示未初始化备用槽位与共享追加时合法的单槽位写入边界。具体字段约束与接口契约以壳源码为准。storage 本身不整体放入 UnsafeCell，避免共享读期间创建覆盖旧元素的整体独占引用。

直接复用 pi_arr 分段存储，不再叠加本库的另一种 Vec。cargo tree -e features 核实当前未启用 rc。已查阅 pi_arr 0.33.0 的 SegmentedArr::load_alloc_ptr/get/slice 与 OwnedSlice：默认模式增长分段桶不移动旧元素，元数据借用不创建覆盖已有元素的整体可变引用。新增 Slot::default 声明补齐底层分配要求，但只构造未初始化存储，不要求 T: Default。

共享追加必须只经 Slot::value.get() 写入未发布尾槽位，不创建 &mut Slot 或整个 storage 的独占引用。用户新增要求：参考 SegmentedArr 提供 unsafe load(&self, index) -> Option<&mut T> 与 unsafe load_unchecked(&self, index) -> &mut T；前者检查已发布长度，后者由调用方保证已同步观察到有效索引。二者只借出目标 T，不分配、不取得提交锁；引用期间调用方排除目标元素所有冲突引用和并发访问，包括迭代引用。已有安全 get/get_mut 不改名或更改返回类型。Mutex 解锁/加锁传递前序提交的写入，长度 Release 发布与读者 Acquire 观察使整个前缀可见，不能只论证最后一个槽位。容器边界现已声明 Send for T: Send、Sync for T: Send + Sync，证明紧邻 unsafe impl；storage 私有，底层 Slot 不声明 Sync。单线程泛型能力不收紧。真实实现仍须核验是否遵守这些别名、所有权与发布约束，声明不代替 Miri 或并发验收。

已补齐私有 RemovedValues 守卫：独占借用旧 SegmentedArr，持有两个不重叠的待析构索引范围。范围保留映射为 0..start、end..old_len；容器 Drop 映射为 0..old_len 和空范围。守卫不分配；显式 drain 先推进游标再启动用户析构，首个 panic 展开时由守卫 Drop 接续，旧存储所有者最后释放。定位已分配槽位不得引入额外分配或可失败的边界逻辑；所有权提交阶段只能使用准备阶段已证实有效的索引。守卫声明、流程和配对实现均已完成，析构 panic 接续已通过验收。

技术闭包：壳中 SharedIter 持有 pi_buckets::BucketIter<Slot<T>>，ExclusiveIter 持有 pi_arr::SegmentedIterMut<Slot<T>>；芯返回这两个真实类型，公共入口仍返回 impl Iterator。只逐槽投影已初始化 T，禁止跨分配伪造切片或将 Slot 切片重解释为 T 切片。当前底层 settle/remain_settle 会先移出存储且可能在安装前失败，不能直接当作本库强准备失败保证的实现；应准备独立目标存储，按本库已定顺序转移和安装。

allocated_slots 的可行枚举使用 pi_arr 0.33.0 的公开 SegmentedArr::segments 与 pi_buckets 0.5.0 的 BUCKETS、SKIP、MAX_ENTRIES、Location::{bucket_len,bucket_capacity}，新增同版本直接依赖；不访问私有桶表、不猜桶数。每个桶的完整绝对区间独立枚举，缺桶跳过，不把一次 len 当容量；具体边界与伪码单源在 AppendVec::allocated_slots。

32/64 位均使用 checked_add；MAX_ENTRIES 是索引，尾后为 MAX_ENTRIES + 1（按公开常量可在两种位宽表示）。不能先把连续长度加全部理论桶容量。尾桶绝对尾后不可表示时停止扫描，因为 push 必须在分配前拒绝该整桶；这也保证实际物理总数可由 usize 精确计数。ZST 的物理槽位同样受此计数约束，不以字节数为零绕过。桶偏移先减 SKIP，再做受检前缀加法，不能用倍增尾桶长度产生 32 位中间溢出。此规则属于既有容量溢出失败契约，未引入计数饱和或截断语义。

## 原暂停问题如何收敛

没有预留索引就没有未提交 Entry、取消空洞或前驱自旋等待；没有第二套长度就没有预留/发布计数同步问题。只需写入完整值后发布一个长度。共享迭代取得长度快照，不读尾部备用槽位；独占迭代只覆盖同一初始化前缀。清理和析构只处理容器拥有的已初始化值。

构造、写入、元素访问、迭代、搬移和析构现均已实现；原三个测试保留并改用安全 push，已随完整测试运行通过。旧预留实现与双容器测试假设已不适用，不作为新协议实现保留。

## 已收敛的失败与边界契约

1. 已确认：扩容允许在提交锁内进行，不承诺固定提交耗时。先完成所有可能展开的边界检查与槽位准备，再执行不可展开的移入和长度发布。准备阶段失败不改变已发布长度或旧值，允许增加备用槽位。
2. 已确认：准备阶段展开造成提交锁中毒时，后续 push 基于上述不变量恢复并清除中毒，不要求用户 clear。尚未移入的 value 在释放锁后析构；OOM abort 不保证恢复。完整流程单源在 AppendVec::push。底层 pi_arr::load_alloc_ptr 具备稳定地址的槽位分配能力，Slot 的默认初始化必须是无用户代码的未初始化存储；具体实现及安全证明仍未完成。
3. 已确认：slice 与 retain_range_and_compact 均严格要求 start <= end <= len，不裁剪；非法范围在读取元素或分配、移动、析构前 panic，容器不变。slice 以入口一次 Acquire 长度快照校验；合法空范围允许，范围保留操作的空范围表示移除全部值。
4. 已确认：重建先准备新存储与回收所需全部资源，准备阶段失败时旧容器不变；随后在无可展开操作的所有权提交段安装新存储与新长度，最后才析构被移除值。用户 Drop panic 时容器已是有效结果（clear 为空），局部守卫负责剩余值，避免重复析构；双重 panic 按 Rust 规则终止。完整流程单源在 AppendVec::retain_range_and_compact，其他入口只写映射差异。Drop 自身不准备新存储、不分配，使用同一撤销所有权规则回收。用户确认重建按精确目标槽位数执行：make_contiguous 为 len + additional，范围保留为 range.len() + additional，clear 为 additional；不隐式沿用旧备用容量，旧分段桶回收，clear(0) 安装无槽位空存储。目标计数为可访问槽位而非分配器字节，ZST 也使用有限计数；加法溢出在修改前失败。
5. 已确认 allocated_slots 接受并发近似观察，不取得提交锁、不分配；可漏计统计期间新分配的桶，不与单独的 len 保证一致快照，不能作为访问边界或免分配保证。无并发写入且操作完成后应准确。枚举路径和计数边界现已核实，不能以近似为由返回无依据计数。
6. 用户确认直接使用 pi_arr::SegmentedArr，容器字段、RemovedValues 字段及构造参数和芯签名均使用具体类型，不使用受 rc feature 切换影响的 Arr 别名。稳定地址前提由具体分段类型表达。容器边界的线程 trait 已声明，不增加全局泛型约束。

当前没有需用户新增选择的局部设计待定；全部业务芯保持明确失败桩。没有待定的业务流程不再使用旧发布协议解释。

## 风险与验收映射（待实现后执行）

验收以公开壳入口为主；内部纯边界计算可补单测，不能代替跨调用和 panic 路径。当前仅接通类型验证，以下业务验收尚未执行。

| 风险 | 壳/声明落点 | 必须观察的验收 |
| --- | --- | --- |
| 多线程提交与旧引用失效 | AppendVec::push/get/iter、Send/Sync impl | 多写者索引唯一且为完整前缀；读者持有旧引用跨越桶增长；Acquire 读到的整个前缀有效；可同步的非 Copy 值跨线程恰好一次析构 |
| 线程 trait 边界 | AppendVec doctest、lib::tests::thread_trait_bounds | String 为 Send + Sync，Cell 为 Send 非 Sync，Rc 非 Send，SyncOnly 非 Send 也不能使容器 Sync；Rc 单线程类型可用，不要求 Default |
| 共享修改别名 | unsafe load/load_unchecked、SharedIter/ExclusiveIter | Miri 检验合法不重叠访问及独占迭代多引用并存；并发追加不产生覆盖旧元素的整体可变引用；unsafe 错误调用不列为库需安全支持的场景 |
| 准备失败、锁中毒 | push、FailurePoint 两个 Append 点 | catch_unwind 后旧值/len 不变、返回索引无空洞；准备后失败允许多出备用桶；下一次 push 自动恢复；待移入值的 Drop 通过 try_lock 观察锁已释放 |
| 严格范围 | slice/retain_range_and_compact | 反向、越界、最大边界在分配/移动/析构前失败；0..0、len..len 合法；slice 排除随后追加值 |
| 精确重建及准备失败 | make_contiguous/retain_range_and_compact/clear、两个 Rebuild 点 | 保留顺序，精确目标槽位、清空旧桶、clear(0) 零槽位；两点 panic 后原值/顺序/len/容量不变；普通值与 ZST 均覆盖 |
| 析构展开所有权 | RemovedValues::new/drain/Drop、AppendVec::drop | 首个 Drop panic 后结果已安装且可继续使用；守卫接续，移除值及保留值最终各析构一次；空区间、整段和两段移除；双重 panic 单独子进程验收 |
| 统计误差与整数边界 | allocated_slots、push 容量准备 | 静止时精确含备用槽；并发观测不加提交锁、不分配；多桶及缺桶扫描；纯边界模型覆盖 BUCKETS 尾桶、MAX_ENTRIES + 1、前缀加法溢出和整桶总数溢出，32/64 位均验证，不实际分配巨量内存 |

测试专用接线在壳声明：FailurePoint、with_failure、fail_if_requested，配对芯已实现并通过失败注入验收。实现须按 push 和 retain_range_and_compact 的流程标注接入准备点；三个重建入口均适用重建失败点。配置只作用于当前线程，以 RAII 在展开时恢复，不能持有全局锁运行闭包；禁止在无展开的所有权提交段注入。底层 OOM abort 不通过捕获 panic 模拟为可恢复。类型验证可独立运行，其余待业务实现完成再执行。

## 最近独立轻审与实现交接

独立 zcode-reviewer 最终复审通过：完整 src、Cargo.toml 与本设计稿，无阻塞设计待定。主代理尚未取得 lint 机械检查结果：本机 CLI 自动安装所需 nightly-2025-03-04-x86_64-pc-windows-msvc，分别在 120 秒与 600 秒超时；审查人工核对适用规则后将其作为非阻塞环境证据限制。当前 cargo check --all-targets 与格式检查通过，cargo test --doc 为 22 项通过（18 个 no_run 仅编译），git diff --check 通过；此前 4 项 doc 检查记录是历史结果。用户已同意轻审后实现。实现根为当前项目根，冻结设计壳 src/append_vec.rs，芯 src/append_vec_inner.rs 与验收测试按既定契约实现；发现需改设计的情况须返回 zcode 更新与复审。

## 当前实现验收

zcode_impl 复杂路线完成全部 23 个工作项，外部 TODO 扫描与独立实现审查通过。当前 src 无 todo!、unimplemented! 或设计待定标记，芯、迭代器及失败注入均已实现。主代理最终复核：cargo test 为 36 个单元测试与 22 个文档测试通过（18 个 no_run 仅编译）；覆盖多写者发布、旧引用稳定、锁中毒恢复、失败值锁外析构、重建回滚、用户析构 panic 接续、双重 panic 子进程终止、ZST 与容量边界。cargo check --all-targets 无错误或警告；cargo fmt -- --check、git diff --check 通过。未取得 Miri 或额外 64 位目标运行证据，zcode lint 仍未取得结果，不宣称这些检查通过。

下文设计阶段的“桩、暂停、尚未测试”等措辞均为历史证据，已由本节当前实现结果取代；公共契约保持已审基线。未提交或发布。

## 设计阶段验证（历史）

工作目录 D:/dev/pi_append_vec。修改前 cargo check --all-targets 通过，5 类设计桩相关 dead_code 警告。修改后 cargo check --all-targets 通过：lib 7 条、lib test 12 条 dead_code 警告（其中 7 条重复）；新增来源为两个迭代游标字段和测试专用失败注入声明，未抑制警告。cargo fmt -- --check 通过（无输出）。cargo test --doc 通过：4 passed、0 failed，含一个正向类型示例和三个 compile_fail 反例，不构造容器、不进入业务桩；lib 中正向线程类型用例已随 all-targets 编译。初次检查发现迭代器配对方法遗漏，补齐声明后重验通过。原三个业务测试只编译，不运行。此前旧结构测试通过不作为新结构的功能证据。本轮为按既有轻审反馈修正后的主 agent 自查，未发起独立复审，不能据此声称轻审通过。

优先尝试 checkRound，但工具初始化报“无法安装依赖 (Unable to resolve the latest dependency version)”，未执行该 API；改用实际 Cargo 命令取得上述原输出。git diff -- Cargo.lock 为空，依赖直接声明未改动锁文件。32 位目标编译、Miri、并发与失败注入业务验收均待实现后验证；当前 32/64 位边界结论来自公开常量和受检运算路径的源码核实，不冒称跨目标运行通过。

依赖 pi_arr 0.33.0、pi_buckets 0.5.0、pi_share 2.1.0；Cargo 元数据移除 lock-free 承诺。保留用户现有 pi_share 依赖。
