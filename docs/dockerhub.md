# Docker Hub 镜像发布

独立的 `Publish Docker Hub images` Actions 任务从已有版本标签重新构建
`geoledger` 和 `geoledger-console`，发布 Linux AMD64/ARM64 镜像、SBOM 和
构建来源信息，不触发 SDK 发布。现有 GHCR 发布流程继续保留。

1. 在 Docker Hub 的账号或组织中准备 `geoledger`、`geoledger-console` 仓库，
   需要匿名拉取时将它们设置为 Public。
2. 创建具有目标仓库写入权限的 Docker Hub Access Token。
3. 在 GitHub 仓库 Settings → Secrets and variables → Actions 中添加
   `DOCKERHUB_USERNAME`（登录用户名）和 `DOCKERHUB_TOKEN`（Access Token）。
   发布到组织时，登录用户须拥有组织仓库的写入权限；不要把 Token 提交到代码。
4. 在 Actions → Publish Docker Hub images → Run workflow 中选择 main，
   namespace 填 Docker Hub 用户名或组织名，release_tag 填已有版本标签，
   如 `v0.3.0-alpha.1`。
5. 两个构建任务成功后，用 `docker pull 命名空间/geoledger:0.3.0-alpha.1`
   和 `docker pull 命名空间/geoledger-console:0.3.0-alpha.1` 验证。

任务不会发布或覆盖 latest。存储格式变化仍需遵循版本契约，更新镜像不会
自动迁移旧数据库。工作流输入版本必须与标签下包版本一致。
