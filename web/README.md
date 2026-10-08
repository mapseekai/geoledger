# GeoLedger Console

独立的 Next.js 管理应用。浏览器通过 HTTP 访问此项目；Next.js 的 Node.js
服务端使用 `@geoledger/client` 业务 SDK 连接 GeoLedger gRPC。Web 与业务服务可独立构建和部署。

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
随机 `GL_WEB_SESSION_SECRET`（例如 `openssl rand -base64 48`）。凭证保存在本地环境配置或秘密管理系统中。

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

Web 通过统一业务服务访问数据；切换 SQLite/PostGIS 时在 GeoLedger 服务端配置存储后端。
Web 镜像以非 root 用户运行，包含 standalone 产物、SDK 和本地字体/静态资源。
构建使用源码和锁定依赖，运行时通过环境配置注入密钥和业务服务地址。

## 开发结构

- `src/app/api`：登录/退出与已认证浏览器业务接口。
- `src/lib/operations.ts`：严格白名单请求到 SDK 业务方法的映射。
- `src/components`：浏览器页面交互，通过 HTTP 调用服务端业务接口。
- `src/components/ui`：shadcn/ui 源组件，可维护和定制。
- `src/app/globals.css`：管理后台主题与响应式布局。

UI 请求版本号使用十进制字符串，服务端转换为 SDK bigint。要素内容使用原始
GeoJSON 文本，展示/补齐 ID 使用 lossless-json；保持原始数字的精度。
访问令牌放入加密、签名的 HttpOnly/SameSite=Strict Cookie，会话最多 8 小时，
HTTPS 下启用 Secure。控制台令牌上限为 2000 个 ASCII 字符；较大的企业 JWT
需由身份提供方精简 claims，或通过 SDK 使用。所有业务请求仍由 GeoLedger 检查权限与令牌有效性。

待确认的发布意图（项目、工作区、版本、说明、请求 ID）暂存在本标签页的
sessionStorage，保存恢复发布所需的请求标识和版本信息。断线、刷新和会话过期后可原样重试；
主动退出会清除此记录，退出前请先确认发布结果。写操作由用户明确发起，发布重试复用原请求。

## 验证

```sh
npm --prefix web test
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

完整浏览器回归见 [控制台文档](../docs/console.md)。

## 设计来源

管理后台布局：深色侧边导航（项目上下文、数据管理 / 系统管理分组、服务状态与会话），
顶部面包屑与连接状态，内容区由页面标题、统计卡片、项目上下文卡片和表格面板组成。
中性灰白界面，GeoLedger 橙色作为唯一强调色；文字与实心按钮使用深橙 `#cc3a05`
以满足 WCAG AA 对比度，亮橙仅用于标志和指示元素。8px 控件 / 12px 卡片圆角、细边框、
轻阴影；状态与角色使用带圆点的色彩徽标。1024px 以下侧边栏切换为抽屉，640px 以下工具栏与统计卡片紧凑排列。
使用本地打包的 Inter；登录页标题使用开源 Newsreader（中文衬线采用系统字体）。
Logo 使用 [GeoLedger SVG 标志](public/logo.svg)；深色背景中的标志主体跟随文字颜色。
shadcn/ui 源码基于 MIT 许可，参见 [第三方说明](THIRD_PARTY_NOTICES.md)。
