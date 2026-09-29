# Windows CLI 测试指南

测试版本为 **0.1.0-alpha.1**，目标为 **Windows x64 / x86_64-pc-windows-msvc**，面向 Windows 10/11 x64 测试。发布验证在 Windows Server 2022 构建机执行。包内 `gl.exe` 提供版本管理 CLI、PostGIS 接入、HTTP 和 gRPC；Volo Thrift 随 Unix 构建提供。

## 解压与校验

解压发布包后，在包含 `gl.exe` 的文件夹打开 PowerShell。包内包含 `README-WINDOWS.md`、`config.env.example`、`test-data.sql`、`smoke-test.ps1`、`build-info.json`、`SHA256SUMS.txt` 和 `LICENSE`。

```powershell
.\gl.exe --version
.\gl.exe --help
Get-FileHash .\gl.exe -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

`--version` 应显示 `gl 0.1.0-alpha.1`。比对 `gl.exe` 的 SHA256 与 `SHA256SUMS.txt` 对应行；ZIP 的独立 `.sha256` 文件用于校验下载包。包使用静态 C 运行时和内置 SQLite；执行环境使用 Windows 自带系统组件。数字签名状态为 unsigned，按所在组织的软件执行策略运行。

## 离线验证

以下命令由 `init` 创建新的测试仓库，可以先检查程序执行、文件权限与默认作者：

```powershell
.\gl.exe --repo '.\demo-repo' init
.\gl.exe --repo '.\demo-repo' status
.\gl.exe --repo '.\demo-repo' branch draft
.\gl.exe --repo '.\demo-repo' log
.\gl.exe --repo '.\demo-repo' fsck
```

默认作者是 `mapseekai`，格式为 3，仓库文件位于 `demo-repo\.geoledger\repository.sqlite`。命令行 `--author` 优先于 `GL_AUTHOR`。仓库路径可包含中文和空格。

包内 `smoke-test.ps1` 执行同类自动化检查，使用独立临时仓库，并在完成后清理该临时仓库、还原进程环境变量。根据本机脚本执行策略运行：

```powershell
.\smoke-test.ps1 -Executable .\gl.exe
```

## 连接 PostGIS

准备独立开发数据库，安装 PostGIS 扩展，并由有相应权限的角色执行包内 `test-data.sql`。脚本创建 `geoledger_demo` schema、两条道路记录和 `roads` 表。数据库角色需要创建 `_geoledger` schema、安装触发器，以及读写测试表和元数据的权限。

`GL_DATABASE_URL` 使用你实际可访问的 PostgreSQL 连接串；连接用户名使用已创建的数据库角色。`GL_AUTHOR` 是提交作者，可单独配置。PowerShell 中输入连接串并导入刚创建的测试表：

```powershell
$env:GL_DATABASE_URL = Read-Host '输入实际 PostgreSQL 连接串'
$env:GL_AUTHOR = 'mapseekai'
.\gl.exe --repo '.\demo-repo' import roads --schema geoledger_demo --table roads
.\gl.exe --repo '.\demo-repo' status
.\gl.exe --repo '.\demo-repo' log
```

`config.env.example` 是配置模板，通过当前 PowerShell 会话显式设置环境变量。版本操作会安装跟踪触发器，并在提交和恢复期间取得表锁，适合在专用测试表中进行验证。

## 数据和字段测试

在 SQL 客户端执行一次测试编辑：

```sql
UPDATE geoledger_demo.roads SET name = 'road-1-updated' WHERE id = 1;
```

然后在同一 PowerShell 中执行：

```powershell
.\gl.exe --repo '.\demo-repo' diff
.\gl.exe --repo '.\demo-repo' commit -m 'Windows data test'
.\gl.exe --repo '.\demo-repo' add-field roads note --type text
.\gl.exe --repo '.\demo-repo' rename-field roads note memo
.\gl.exe --repo '.\demo-repo' alter-field-type roads memo --type 'varchar(200)'
.\gl.exe --repo '.\demo-repo' drop-field roads memo --discard
.\gl.exe --repo '.\demo-repo' fsck
```

每条字段命令以干净工作副本为前提，并自动生成提交。`restore --discard` 将工作副本恢复为 HEAD；`reset COMMIT_ID --hard` 移动分支并覆盖工作副本。执行恢复前确认目标仓库和提交，提交 ID 取自 `log`。

## HTTP 与 gRPC

以下命令创建本机 HTTP 和 gRPC 监听地址；先在 PowerShell 中配置至少 24 字节的随机令牌，再启动服务：

```powershell
$bytes = New-Object byte[] 32
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
$rng.GetBytes($bytes)
$rng.Dispose()
$env:GL_API_TOKEN = [Convert]::ToBase64String($bytes)
.\gl.exe --repo '.\demo-repo' serve --http 127.0.0.1:7878 --grpc 127.0.0.1:7879
```

服务运行时，在另一个 PowerShell 中输入相同令牌，读取状态：

```powershell
$token = Read-Host '输入启动服务时配置的 GL_API_TOKEN'
Invoke-RestMethod -Headers @{Authorization="Bearer $token"} -Uri 'http://127.0.0.1:7878/v1/status'
```

服务终端使用 Ctrl+C 结束。HTTP/gRPC 共享绑定仓库和 PostGIS 连接；跨机器部署通过 TLS 反向代理保护传输。

## 测试反馈

反馈中附上 `gl --version`、Windows 版本、执行命令、完整错误输出和 `build-info.json`。数据库连接串、密码和 API 令牌请先脱敏。重点检查文件锁、中文路径、连接、数据提交、字段操作、分支合并及恢复结果。
