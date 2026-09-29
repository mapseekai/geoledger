# 快速开始

[项目概览](../README.md) · [功能指南](user-guide.md) · [API](api.md) · [开发说明](development.md)

## 1. 运行 CLI

解压 Windows x64 测试包，在 `gl.exe` 所在目录打开 PowerShell。以下命令统一使用 `.\gl.exe`：

```powershell
.\gl.exe --version
.\gl.exe --help
```

包内提供配置模板、测试 SQL、自检脚本及 SHA256 校验清单。源码构建方式见 [开发说明](development.md#构建)。

## 2. 初始化仓库

```powershell
.\gl.exe --repo '.\demo-repo' init
.\gl.exe --repo '.\demo-repo' status
.\gl.exe --repo '.\demo-repo' log
.\gl.exe --repo '.\demo-repo' fsck
```

`init` 创建仓库目录及 `.geoledger/repository.sqlite`；仓库路径可包含中文和空格。

## 3. 接入 PostGIS

在专用开发数据库中启用 PostGIS，再执行包内 `test-data.sql`（[源码脚本](../scripts/windows-test-data.sql)），创建 `geoledger_demo.roads` 和两条道路记录。连接角色需具备创建 `_geoledger` schema、安装触发器及读写测试表的权限。

```sql
CREATE EXTENSION IF NOT EXISTS postgis;
```

在 PowerShell 设置实际连接串并导入：

```powershell
$env:GL_DATABASE_URL = Read-Host '输入开发数据库的 PostgreSQL 连接串'
.\gl.exe --repo '.\demo-repo' import roads --schema geoledger_demo --table roads
.\gl.exe --repo '.\demo-repo' status
```

`import` 建立数据快照及变更跟踪。示例 SQL 使用新的 schema，测试时在独立开发数据库中执行一次。

## 4. 完成首次提交

在 SQL 客户端或 QGIS 中修改记录，例如：

```sql
UPDATE geoledger_demo.roads SET name = 'road-1-updated' WHERE id = 1;
```

随后查看差异并提交：

```powershell
.\gl.exe --repo '.\demo-repo' diff
.\gl.exe --repo '.\demo-repo' commit -m '更新道路名称'
.\gl.exe --repo '.\demo-repo' log
```

## 中心版

中心版入口为 `gl-center`，使用独立 PostGIS 数据库和服务专用写入角色。管理员先在该数据库的 `public` schema 启用 PostGIS：

```sql
CREATE EXTENSION IF NOT EXISTS postgis;
```

使用中心版二进制，或从项目根目录执行 `cargo build -p geoledger-center --locked`，生成程序位于 `target/debug`。
在 `gl-center.exe` 所在目录打开 PowerShell（macOS/Linux 将命令改为 `./gl-center`）：

```powershell
$env:GL_CENTER_DATABASE_URL = Read-Host '输入独立开发数据库的 PostgreSQL 连接串'
.\gl-center.exe tokens --out .center-tokens.json alice bob
$env:GL_CENTER_TOKEN_FILE = (Resolve-Path .center-tokens.json).Path
.\gl-center.exe migrate
.\gl-center.exe serve --listen 127.0.0.1:7881
```

`tokens` 使用系统安全随机数生成每个用户的令牌，并创建新文件。限制该文件的访问权限，分别向 alice 和 bob 分发各自令牌；
服务启动时载入映射，调整后重启生效。`migrate` 在事务中创建 `_geoledger_center`，重复执行核对中心格式版本。

启动后打开 `http://127.0.0.1:7881/`，进入浏览器协作测试台。此地址由上述 `serve` 命令创建。

1. 在窗口 A 输入 alice 的令牌，依次创建项目、数据集，并通过“设置项目成员”将 bob 设为 editor。
2. 在窗口 B 输入 bob 的令牌，填入相同项目和数据集 ID。两个窗口分别创建自己的草稿。
3. 选择“保存要素”，编辑 GeoJSON；通过“查看草稿差异”核对后发布。数据、历史和发布记录在同一事务内保存。
4. 同一要素的独立属性修改可自动合并；同字段冲突通过“查询合并冲突”和“分批解决冲突”处理，再发布。

生成请求示例会读取左侧 ID 与版本，结果自动回填上下文。令牌仅保留在页面内存中。中心版通过草稿 API 向服务管理的数据集录入要素，
已发布要素位于 `_geoledger_center.features`，几何为 PostGIS geometry，属性为 JSONB。

服务角色负责该 schema 的写入；数据库外部使用者按需配置只读权限。跨机器访问通过 TLS 反向代理同时提供页面和 `/api/center/`。
API 细节见 [中心接口](api.md#中心版)，本地 `gl` 与中心 `gl-center` 的数据分别维护。
