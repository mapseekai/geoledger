# GeoLedger Console

独立的 Next.js 管理应用。浏览器通过 HTTP 访问此项目；Next.js 的 Node.js
服务端使用 `@geoledger/client` 业务 SDK 连接 GeoLedger gRPC。Rust 服务不再嵌入页面。

技术栈按 npm stable 锁定：Next.js 16.4、React 19.3、TypeScript 7、Tailwind CSS 4.3，
shadcn/ui 源码组件（Radix）与 lucide 图标。以 `package-lock.json` 为可复现安装依据。

## 本地启动

在仓库根目录执行（Node.js 22）：

```sh
npm ci --prefix sdk/ts
npm --prefix sdk/ts run build
npm ci --prefix web
cp web/.env.example web/.env.local
```

编辑 `.env.local`：设置 `GL_WEB_ENDPOINT` 为 GeoLedger 的 gRPC 地址；
设置 `GL_WEB_ORIGIN` 为浏览器实际访问的完整 origin；生成并填入至少 32 字符的
随机 `GL_WEB_SESSION_SECRET`（例如 `openssl rand -base64 48`）。不要提交凭证。

```sh
npm --prefix web run dev
```

访问 `http://localhost:3000`，输入管理员分配的令牌。默认后端地址为
`http://127.0.0.1:7882`。SDK 是工作区的本地包依赖；修改 SDK 后重新构建 SDK。

## 生产运行

```sh
npm --prefix web run build
npm --prefix web start
```

生产服务使用固定的 `GL_WEB_SESSION_SECRET`，多实例共享同一密钥和配置。
`GL_WEB_ORIGIN` 在非 loopback 环境必须使用 HTTPS；通过反向代理终结 TLS，
让 Node 服务只监听内网。SDK 对远程服务可用 `https://` 校验 gRPC TLS 证书。
反向代理应设置请求体大小、连接数和登录速率限制，并保留浏览器的 Origin。

也可从仓库根目录构建镜像：

```sh
docker build -f web/Dockerfile -t geoledger-console .
# 在环境中设置 GL_WEB_SESSION_SECRET，再启动组合：
docker compose --profile console up --build
```

Web 与数据库无直接连接；切换 SQLite/PostGIS 只修改 GeoLedger 的存储配置。
Web 镜像以非 root 用户运行，包含 standalone 产物、SDK 和本地字体/静态资源。
密钥只在运行时传入；构建不需要密钥，也不读取数据库。

## 开发边界

- `src/app/api`：登录/退出与已认证浏览器业务接口。
- `src/lib/operations.ts`：严格白名单请求到 SDK 业务方法的映射。
- `src/components`：页面交互；禁止导入 Node SDK 的运行时代码。
- `src/components/ui`：shadcn/ui 源组件，可维护和定制。
- `src/app/globals.css`：Mistral 参考主题与响应式布局。

UI 请求版本号使用十进制字符串，服务端转换为 SDK bigint。要素内容使用原始
GeoJSON 文本，展示/补齐 ID 使用 lossless-json；不经原生 JSON 数字往返。
访问令牌放入加密、签名的 HttpOnly/SameSite=Strict Cookie，会话最多 8 小时，
HTTPS 下启用 Secure。控制台令牌上限为 2000 个 ASCII 字符；较大的企业 JWT
需由身份提供方精简 claims，或通过 SDK 使用。所有业务请求仍由 GeoLedger 检查权限与令牌有效性。

待确认的发布意图（项目、工作区、版本、说明、请求 ID）暂存在本标签页的
sessionStorage，不含令牌和要素内容。断线、刷新和会话过期后可原样重试；
主动退出会清除此记录，退出前请先确认发布结果。不会自动重发写操作。

## 验证

```sh
npm --prefix web test
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

完整浏览器回归见 [控制台文档](../docs/console.md)。

## 设计来源

参考 [Mistral DESIGN.md](https://github.com/VoltAgent/awesome-design-md/blob/main/design-md/mistral.ai/DESIGN.md)：
暖白、奶油色、橙色强调、8px 按钮 / 12px 卡片、细边框、页底日落色带。
橙色按钮采用深色文字以提高正文对比度；导航的橙色文字使用更深的色阶。
使用本地打包的 Inter；商业字体 PP Editorial Old 不随本项目分发，显示字体
回退为开源 Newsreader（中文衬线采用系统字体）。如持有授权，可自行配置展示字体。
Logo 沿用 GeoLedger 标志，不使用 Mistral 商标。
shadcn/ui 源码基于 MIT 许可，参见 [第三方说明](THIRD_PARTY_NOTICES.md)。
