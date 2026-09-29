# 功能指南

[项目概览](../README.md) · [快速开始](getting-started.md) · [API](api.md) · [开发说明](development.md)

以下沿用快速开始创建的 `demo-repo` 与 `roads`，并使用同一 `GL_DATABASE_URL`。示例为 PowerShell；macOS / Linux 中将 `.\gl.exe` 换为已安装的 `gl`。

## 查询数据与历史

| 命令（接在 `.\gl.exe --repo '.\demo-repo'` 后） | 功能 |
|---|---|
| `status`、`diff` | 查看工作副本状态与变更 |
| `log`、`show HEAD` | 查看提交历史与提交详情 |
| `show HEAD --dataset roads --key 1` | 查看指定版本的记录 |
| `schema roads --reference HEAD` | 查看字段结构 |
| `diff --from main --to draft` | 比较两个分支的版本差异 |
| `branch`、`reflog` | 列出分支及引用变更记录 |

版本引用使用 `HEAD`、分支名或 `log` 返回的完整提交 ID。适用表采用单列主键、默认列排序规则，以及适配器声明的 PostgreSQL 标量或 geometry 字段；几何历史保留 SRID、Z/M 和坐标表达。

## 记录提交

通过 SQL / QGIS 编辑注册表后，运行 `diff` 查看新增、修改和删除，再用 `commit -m '说明'` 保存。一次提交可同时包含记录与字段变化。每个仓库共用一个 PostGIS 工作副本，版本操作通过事务和表锁协调写入。

## 字段演进

在干净工作副本上执行，每条字段命令自动生成提交：

```powershell
.\gl.exe --repo '.\demo-repo' add-field roads note --type text
.\gl.exe --repo '.\demo-repo' rename-field roads note memo
.\gl.exe --repo '.\demo-repo' alter-field-type roads memo --type 'varchar(200)'
.\gl.exe --repo '.\demo-repo' drop-field roads memo --discard
```

新增命令创建可空字段；删除用于普通非主键字段，`--discard` 确认删除字段及其当前值。改名保留字段身份；类型转换由数据库执行显式 cast，转换前的值保存在历史中。索引、约束及可恢复的字面量默认值参与结构校验。

也可先在 SQL / QGIS 中完成字段增删、改名或类型调整，再通过 `status`、`diff`、`commit` 记录。结构变化的提交与恢复按整表扫描或回填处理，跟踪触发器持续启用。

## 分支与合并

分支切换和合并以干净工作副本为前提，切换会将注册表更新至目标分支：

```powershell
.\gl.exe --repo '.\demo-repo' branch draft
.\gl.exe --repo '.\demo-repo' switch draft
# 在 SQL / QGIS 中编辑记录后提交：
.\gl.exe --repo '.\demo-repo' commit -m '调整道路属性'
.\gl.exe --repo '.\demo-repo' switch main
.\gl.exe --repo '.\demo-repo' merge draft
```

支持快进、三方字段级合并、独立字段新增，以及一侧字段改名与另一侧记录编辑的合并。几何按完整字段值参与比较。

记录冲突用 `conflicts` 查看；将以下 `1` 替换为实际冲突主键后选择结果：

```powershell
.\gl.exe --repo '.\demo-repo' conflicts
.\gl.exe --repo '.\demo-repo' resolve roads 1 --take theirs
.\gl.exe --repo '.\demo-repo' merge --continue
```

`--take` 可选 `ours`、`theirs`、`base`、`delete`、`custom`；自定义结果通过 `--record` 指向完整记录 JSON。全部冲突解决后统一提交；取消用 `merge --abort`。字段定义冲突先在分支上对齐结构，再发起合并。

## 恢复与校验

| 命令 | 操作效果 |
|---|---|
| `restore --discard` | 确认丢弃未提交修改，将工作副本恢复为 HEAD |
| `reset COMMIT_ID --hard` | 确认移动当前分支并覆盖工作副本 |
| `revert COMMIT_ID` | 通过新提交撤销指定单父提交 |
| `recover` | 核对数据库操作标记，协调中断操作的仓库状态 |
| `fsck` | 校验对象、提交图和数据树 |

以上命令同样添加仓库前缀。执行恢复前确认仓库及目标提交，`COMMIT_ID` 取自 `log`。数据与字段结构共同恢复，表 OID 和跟踪触发器保持完整。

## 中心版

1. 创建项目后，创建者成为 owner。owner 可通过 `set_member` 设置 owner/editor/viewer；项目始终保留至少一位 owner。
2. owner/editor 创建集合与自己的工作区。工作区固定 `base_revision`，读取该修订的历史要素并覆盖自己的增量。
   其他人的发布只影响已发布视图；各人的工作区列表、详情、差异和编辑受成员身份及所有权检查。
3. 使用 `save` 一次提交最多 100 个完整 Feature 或删除标记；请求提供 `expected_workspace_version`。
   批次原子保存，每工作区最多 1000 个增量。编辑器应保留从该草稿读取的其余属性；缺失属性代表删除，JSON null 代表显式空值。
4. 检查 `diff` 后 `publish`。项目短时行锁串行发布；基于固定基础、当前 HEAD 与草稿进行字段级三方合并。
   不同属性可合并，同值修改可合并；几何作为整体，更新/删除与不同的新建内容产生冲突。
5. HTTP 409 返回冲突预览、HEAD 和工作区版本。`conflicts` 按游标获取完整冲突；`resolve` 携带这两个版本，
   每次确认最多 100 个冲突要素的最终 Feature（或 null）。继续使用返回的新工作区版本处理余下冲突并发布。
   `rebase` 可一次解决全部剩余冲突，同时把草稿基线更新到当前 HEAD。正式版本变化或草稿再次编辑后，解决结果重新校验。
6. 发布成功可用相同 `request_id` 和完全相同的请求重试并取得同一结果；复用 ID 更改请求返回 409。
   `discard` 关闭自己的草稿。保存和丢弃均保持已发布数据原样。
7. `history` / `commit` 查询提交及前后值，`features` 的 `revision` 参数读取时态历史。
   `restore` 为指定提交创建一个反向修改草稿，以该提交为固定基础；查看、解决冲突并发布后形成新提交。

中心几何为 SRID 4326 的有效二维或带 Z 值 GeoJSON geometry（也可为 null），通过 XDR EWKB 保存坐标顺序和几何表达。
坐标限经度 [-180,180]、纬度 [-90,90]。属性保留 JSON 数字、布尔、数组、对象与 null 类型；要素 ID 为稳定字符串。
历史几何和草稿几何具有 GiST 索引；bbox 查询先排除被草稿覆盖或删除的基础要素，再进行空间过滤。
