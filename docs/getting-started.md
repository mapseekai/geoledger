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
