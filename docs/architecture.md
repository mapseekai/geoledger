# 架构与一致性约束

## 1. 版本仓库与工作副本分离

PostGIS 是可编辑工作副本；本地不可变对象库和分支引用保存历史。一个提交指向 Snapshot；Snapshot 映射逻辑 dataset ID 到 schema 对象、记录树根和计数。记录主键在当前单主键限制下是稳定的文本键。

对象类型包括 `record/v1`、`schema/v1`、`tree-node/v1`、`snapshot/v1`、`commit/v1`。对象 ID = BLAKE3(固定域 + 对象类型长度 + 对象类型 + 编码字节)。先编码和哈希，再压缩。类型隔离避免不同对象类型共享 ID。读取时校验类型、解压大小和哈希。

编码使用结构化 serde JSON、BTreeMap 键序和显式格式版本，不声明 RFC 8785 兼容。SQL 数值、时间等属性保存数据库规范化文本与 schema 类型信息，避免 JSON 浮点精度损失。几何保存 XDR EWKB 十六进制，保留 SRID / Z / M 等编码；不进行 ST_Normalize、坐标舍入、环反转或投影转换。比较的是表达值，不是 ST_Equals 拓扑等价性。

## 2. 持久化数据树

以记录键为查找顺序、BLAKE3(key) 为确定性优先级构建 treap。修改路径生成新节点，未变子树复用，旧提交无需复制全表。初次导入使用有序流的 Cartesian stack 构建器。树深超过保护阈值会拒绝。

相同根直接跳过 diff；相同键根递归比较。当插入/删除导致根结构不同，当前实现会加载相关子树进行比较，因此不能声称所有 diff 都是 O(changes log N)。大规模基准、分页流式 diff 和对象批写是后续工作。

## 3. PostGIS 跟踪与事务

行触发器分别记录 OLD / NEW 主键，使主键改变能够形成删除旧记录与新增新记录。dirty set 去重，不直接把触发器操作码当作最终变化；应用层读取当前行后与 HEAD 比较，因此无效更新或改回原值不会生成虚假 diff。

事务取得仓库 advisory lock，并按稳定顺序对注册表取得 SHARE ROW EXCLUSIVE 锁。该锁允许普通 SELECT，但阻止其他数据写入，使 dirty 集合、当前行、引用移动之间具备明确的提交边界。SQL 标识符双引号转义，数据值参数绑定。工作副本写入不关闭触发器；完成后在同一事务内清理 dirty set。

会核对 schema 指纹、表绑定以及触发器是否存在/启用。TRUNCATE 通过语句触发器拒绝。角色有权限直接篡改元数据或临时绕过触发器时，仍然超出当前可信边界；不能把该方案视为审计防篡改产品。

## 4. 两套存储不是一个原子事务

SQLite 和 PostgreSQL 不共享事务。正常更新按以下顺序执行：

1. 保持 PostgreSQL 事务和表锁，计算/应用变化，准备数据库 operation UUID 与目标 HEAD。
2. 在 SQLite 持久化新增不可变对象和 PendingOperation；此时不公开新的分支状态。
3. 提交 PostgreSQL：数据、dirty 清理、operation 标记同事务提交。
4. 原子更新 SQLite 仓库状态并删除 PendingOperation。

恢复检查数据库 operation 标记：匹配 pending 的 operation 且 HEAD 对应，则完成本地状态；未匹配且数据库仍在原 HEAD，则撤销本地 pending；其他不一致拒绝猜测。调用期间的不确定数据库提交错误必须保留 pending，不盲目执行第二次写入。

这是可恢复协议，不是跨系统 ACID / 2PC 保证。基础约束失败回滚测试不等于 kill -9、断电、磁盘满、数据库 failover 等完整故障矩阵；投产前必须完成故障注入验证、孤儿对象管理和备份恢复演练。

## 5. 合并状态

常见三方合并以 BASE / OURS / THEIRS 对比。普通字段独立判断，geometry 字段作为原子值。同键不同新增、改删冲突保留整记录冲突。多个最佳共同祖先的 criss-cross 合并拒绝，暂不构建递归虚拟祖先。

冲突与候选树单独持久化，HEAD 和 PostGIS 工作副本维持 OURS。resolve 在候选树上操作；自定义值经 PostGIS 类型规范化。continue 要求工作副本仍干净且全部冲突解决，才统一应用。abort 只删除合并状态，不覆盖用户数据库编辑。

## 6. 扩展接口

- `WorkingCopyProvider` 创建受控工作副本事务；能力通过适配器实现，不进入版本树代码。
- `WorkingCopyTransaction` 提供 inspect/register/scan/read/dirty_keys/normalize/write/marker 等协议。
- `ObjectStore` 负责按类型存取不可变对象，当前实现是带 Zstd 的 SQLite。
- `DatasetKind`、版本化 schema、`Cell::Blob(ObjectId)` 为文件、栅格、点云等后续模型留位。

未来 GeoPackage / GeoParquet / 栅格仍需实现自己的编码、能力声明、变更发现与事务/恢复语义。不能仅实现 read/write 就宣称与 PostGIS 等价。当前应用层也拒绝一个仓库混合多种工作副本 provider；这需要单独设计协调协议。
