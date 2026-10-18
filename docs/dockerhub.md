# Docker Hub 镜像发布

`geoledger` 是统一的 Linux 镜像，包含 GeoLedger 服务端、Web 控制台和远程 `gl` CLI，提供 AMD64 与 ARM64 两种架构。控制台通过容器内回环地址连接私有 Session HTTP 传输；SQLite 数据持久化到 `/data`，PostGIS 使用配置的外部数据库。

```sh
export GL_WEB_SESSION_SECRET="$(openssl rand -base64 48)"
docker run -d --name geoledger --restart unless-stopped --stop-timeout 60 \
  -p 127.0.0.1:3000:3000 -p 127.0.0.1:7881:7881 -p 127.0.0.1:7882:7882 \
  -e GL_WEB_SESSION_SECRET -e GL_WEB_ORIGIN=http://localhost:3000 \
  -v geoledger-data:/data zwishing/geoledger:0.3.0-alpha.5
```

浏览器访问 `http://localhost:3000`。首次启动的随机管理员密码位于服务日志提示的 `/data/initial-admin-<id>.json`，首次登录要求修改密码。对外部署时配置实际 HTTPS origin 和 TLS 网关，详见 [生产运行](production.md)。

本地构建使用 `docker build -t geoledger:local .`。发布流程将完整构建阶段导出为 OCI，扫描 Rust、Node 与编译工具依赖，再从已验证的构建产物组装统一运行镜像。运行镜像通过双架构扫描后按原摘要上传，保存 SBOM、构建来源和镜像清单。

源码加镜像的预发布使用 [Source prerelease and Docker Hub](../.github/workflows/source-docker-release.yml)：填写当前 main 的完整候选 SHA 和新版本标签，门禁全部通过后自动公开 GitHub prerelease。此路径不发布 npm/PyPI 包或独立二进制；SDK 调用方从同一版本标签的 `sdk/` 源码构建，并与服务端同步升级。生产使用 Release 的镜像清单中的完整 digest，操作见 [生产运行](production.md)。

Actions → Publish Docker Hub images 支持按已有版本标签构建：namespace 使用目标 Docker Hub 账号或组织，release_tag 如 `v0.3.0-alpha.5`。配置 `DOCKERHUB_USERNAME`、`DOCKERHUB_TOKEN`，登录用户需拥有目标仓库写权限。该流程独立发布容器镜像；版本标签与源码中的版本一致。

预发布使用明确版本标签，数据卷按当前存储格式管理，更新前保留可验证的备份。
