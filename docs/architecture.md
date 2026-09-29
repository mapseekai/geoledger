# 架构与一致性

## 版本历史与工作副本

PostGIS 提供可编辑工作副本；本地对象库和分支引用保存历史。提交指向 Snapshot，Snapshot 映射 dataset ID 到 schema、记录树根及记录数。数据仓库、结构和对象采用统一 GeoLedger 格式 3，定义见 [数据格式](format.md)。

对象 ID 由 BLAKE3 对固定域、类型长度、类型和编码字节计算，先编码、哈希，再压缩。对象读取校验类型、解压大小与哈希。编码使用结构化 serde JSON 和 BTreeMap 键序。属性保存数据库规范文本及字段类型，几何使用小写 XDR EWKB 保留 SRID、Z/M 与坐标表达；版本差异按保存的值比较。

## 持久化树与批处理

记录树使用按键排序、以 BLAKE3(key) 为确定性优先级的 treap。修改路径生成新节点，共享子树按对象 ID 复用。初次导入采用有序流和 Cartesian stack 构建。差异遍历按优先级展开有序任务栈，在解码前跳过相同子树，遍历全部差异以获得精确总数，并按预览预算解码记录。

压缩上下文和 SQL 语句复用；对象解压按帧大小分配并执行 64 MiB 容量检查。SQLite 使用 WAL、synchronous=FULL、64 MiB 主缓存目标和 256 MiB mmap 窗口。结构全量扫描先写文件型临时对象表，再按哈希排序写入正式表，临时表使用 16 MiB 页缓存。批写 savepoint 位于原事务内，失败时回滚。

## PostGIS 跟踪

行触发器记录 OLD / NEW 主键，dirty 集合按键去重。应用层读取当前行并与 HEAD 比较，主键修改表现为旧键删除和新键新增。事务取得仓库 advisory lock，再按稳定顺序获取注册表 SHARE ROW EXCLUSIVE 锁，允许普通 SELECT，并在版本操作期间协调写入。

跟踪 schema 为 `_geoledger`，使用 `gl_track_row_v3` 和 `gl_reject_truncate_v3` 保持变更可追踪。普通清空采用 DELETE，触发器持续启用，完成操作时在同一事务内清理 dirty 集合。SQL 标识符正确转义，数据值使用参数绑定。每次操作校验表绑定、字段结构和跟踪触发器。

dirty 主键按 1000 个分页，使用 `dirty_pk_c_v3` 的 C 排序索引。记录按约 8 MiB 分批读取和写入，单条大记录可超过批大小。字段演进通过稳定 ID 与 attnum 对齐，结构提交全量扫描，历史恢复保留表 OID、调整结构并分批回填。

## 两存储协调协议

SQLite 与 PostgreSQL 各自维护事务，GeoLedger 使用可恢复 journal / marker 协议协调：先在数据库事务和表锁内准备变化与 operation UUID；再在 SQLite 持久化新增对象和 PendingOperation；随后提交数据库中的数据、dirty 清理及 operation 标记；最后原子更新 SQLite 分支状态并清除 pending。

恢复时比较 operation UUID 与 HEAD：匹配目标状态则完成本地发布，数据库保持原 HEAD 则撤销本地 pending，其余情况返回 recovery_required 并保留诊断信息。数据库提交结果待确认时保留 pending，后续通过 recover 核对状态。故障注入、备份恢复和审计能力的开发安排见 [开发计划](roadmap.md)。

## 合并与历史恢复

三方合并比较 BASE / OURS / THEIRS，普通字段独立判断，geometry 按原子值处理。同键不同新增、改删冲突保存为记录冲突。合并基点通过可达位传播和父边缓存计算，单个提交解码一次；唯一最佳共同祖先用于三方合并，多基点场景返回明确结果供调用方处理。

结构按稳定列 ID 合并，并预计算投影关系；源结构新增字段使用适配器提供的常量默认值，已有字段的显式 NULL 保持原值。支持独立新增字段以及一侧改名、另一侧编辑记录。结构冲突通过分支定义对齐后再次合并。

冲突与候选树独立持久化，HEAD 和工作副本保持 OURS。resolve 更新候选树，自定义值由 PostGIS 规范化。continue 要求干净工作副本和全部冲突已解决，按目标类型验证候选值表达后统一发布；abort 清理合并状态并保留工作副本内容。

同结构恢复采用先删除全部待替换行、再分批写入目标行的事务顺序，支持唯一值交换。跨结构恢复从对象记录读取历史值，保留表 OID 和跟踪触发器后调整定义、回填记录。恢复操作所需资源与数据量及结构变化范围相关。

## 分层与扩展接口

[WorkingCopyProvider / WorkingCopyTransaction](../crates/core/src/adapter.rs) 定义工作副本事务、表检查、扫描、dirty 键、规范化、写入和标记协议；[ObjectStore](../crates/core/src/object.rs) 定义按类型存取对象的接口。核心版本算法独立于具体数据库和网络传输。

[Application](../crates/app/src/lib.rs) 编排各版本操作；HTTP、gRPC、Thrift 和 CLI 均调用该服务。异步入口通过阻塞工作线程执行同步数据库操作，并共享请求容量、回复编码和执行槽位控制。

当前适配器提供 PostGIS 表与向量工作副本，存储实现采用 SQLite + Zstd。扩展工作副本时分别定义数据编码、能力声明、变化发现、事务和恢复语义，并复用提交图、分支和合并接口。
