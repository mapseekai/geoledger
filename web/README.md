# GeoLedger Console

<img src="public/logo.svg" alt="GeoLedger Console" width="180">

GeoLedger Console 提供空间数据的协作管理界面：项目与数据集管理、GeoJSON 编辑、工作区发布、冲突解决、版本撤销、成员权限和审计查询。浏览器使用 HTTP，Next.js 的 Node.js 服务端通过私有 HTTP Session transport 连接 GeoLedger，数据与权限由业务服务统一管理。

桌面与手机操作见 [控制台教程](../docs/console.md)。[浏览器验证](../docs/development.md#控制台浏览器验证) 可生成登录页、项目页和手机布局截图，输出到 `--screenshots` 指定目录。

## 获取代码

按照 [仓库克隆说明](../README.md#获取代码) 获取完整仓库。以下命令均从仓库根目录执行；`web/` 通过本地包依赖使用 `sdk/ts`。

## 环境准备

- [Node.js 22](https://nodejs.org/en/download) 与随附 npm，和根目录 Dockerfile、主 CI 保持一致。
- 已运行的 GeoLedger 服务及用户账户或 API Key，启动方式见 [快速开始](../docs/getting-started.md)。
- OpenSSL 或 Python 3，用于生成随机会话密钥。

```sh
node --version
npm --version
```

依赖以 `package-lock.json` 为安装依据。技术栈为 Next.js 16.4、React 19.3、TypeScript 7、Tailwind CSS 4.3，组件采用 shadcn/ui 源码与 Radix、cmdk、react-resizable-panels，图标为 lucide。

## 本地启动

### 安装依赖

先构建 TS SDK，再安装控制台依赖：

```sh
npm ci --prefix sdk/ts
npm --prefix sdk/ts run build
npm ci --prefix web
```

### 配置本地环境

首次配置时创建本地文件：

```sh
cp web/.env.example web/.env.local
openssl rand -base64 48
```

将生成的随机值填入 `web/.env.local` 的 `GL_WEB_SESSION_SECRET`。也可用 `python3 -c 'import secrets; print(secrets.token_urlsafe(48))'` 生成密钥。配置文件采用以下字段：

| 字段 | 默认值 / 要求 |
|---|---|
| `GL_WEB_ENDPOINT` | `http://127.0.0.1:7881`，GeoLedger 的 HTTP 地址 |
| `GL_WEB_DIST_DIR` | 可选，隔离验证可设为 `../target/auth-refactor-web/next`，配合 `NEXT_PRIVATE_OUTPUT_TRACE_ROOT` 指向仓库根及 `NODE_PATH` 指向 `web/node_modules`，避免影响运行中的构建产物 |
| `GL_WEB_ORIGIN` | `http://localhost:3000`，与浏览器访问的协议、主机和端口一致 |
| `GL_WEB_SESSION_SECRET` | 至少 32 字符的随机密钥，保存在本地配置或秘密管理系统 |
| `GL_WEB_BASEMAP_URL` | 可选，XYZ 栅格瓦片地址模板（HTTP(S)，含 `{z}` `{x}` `{y}`），如 `https://tile.openstreetmap.org/{z}/{x}/{y}.png`；设置后地图底图增加“在线地图”，CSP 的 `img-src` 与 `connect-src` 仅追加该来源 |
| `GL_WEB_BASEMAP_ATTRIBUTION` | 可选，在线底图署名，如 `© OpenStreetMap contributors` |

### 开发服务器

保持后端运行，在另一个终端执行：

```sh
npm --prefix web run dev
```

终端显示 Ready 后，访问 `http://localhost:3000`。输入用户名和密码，或切换 API Key 登录，进入项目列表；侧栏显示存储后端与“已连接”。此命令运行带热更新的本地开发服务，TS SDK 修改后先重新构建 SDK。

使用其他端口时，同时设置 `GL_WEB_ORIGIN`。例如将其改为 `http://localhost:3001` 后执行：

```sh
npm --prefix web run dev -- --port 3001
```

浏览器始终访问配置中的完整 origin。

## 构建、测试与部署

### 检查与构建

```sh
npm --prefix web test
npm --prefix web run lint
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

生产构建输出至 `web/.next`；浏览器与真实服务联调入口见 [开发指南](../docs/development.md#控制台浏览器验证)。

GeoJSON 文件上传专项浏览器验证：`python3 scripts/test-console-upload.py --url http://localhost:3000 --api-key-file target/auth-refactor-integration/api-key --screenshots /tmp/geoledger-upload-shots`。使用隔离测试服务，验证创建数据集时的文件上传、大于 1 MiB 的文件、多批次导入、中断重试、提交成功后响应丢失的恢复、并发修改保护、精确数字和手机上传入口。 添加 `--feature-bytes 12582912` 可验证单个 12 MiB 要素经 Web/BFF/HTTP 流式上传、查询及发布；这是验证用例大小，不是产品上限。

### 设计规范检查

`npm --prefix web run lint` 使用 Oxlint，并通过 `.oxlintrc.json` 注册 `@shadcn/lint`。组件目录、导入别名和 Tailwind 主题从 `components.json` 自动发现。六条设计规则均作为错误启用，检查未知 class、主题颜色、任意值、组件样式契约、内联样式和静态 class。命令同时拒绝警告，并由 CI 的 Web 检查执行。组件可接受的手写 CSS 类逐项记录在 `no-restyle.contracts` 中，新增样式需先确认 CSS 定义和使用范围。

基础 `src/components/ui/` 组件负责自身样式，按上游建议豁免样式覆盖、任意值和静态 class 检查，继续检查主题颜色及未知 class。`conflict-resolution.tsx` 和 `version-graph.tsx` 使用编译生成类名的 CSS Modules，豁免静态 class 解析；版本图仅允许动态 `left`/`background`，进度条仅允许动态 `transform`。动画工具类由 `tw-animate-css` 提供，全局 CSS 保留减少动态效果的系统偏好适配。规则说明见 [可用规则](https://github.com/shadcn-ui/lint#rules)，配置示例见 [设计系统规则](https://github.com/shadcn-ui/lint/blob/main/docs/design-systems.md)。

### 生产运行

完成构建并配置运行环境后执行：

```sh
npm --prefix web start
```

生产控制台的各实例使用相同 `GL_WEB_SESSION_SECRET` 和 origin 配置，并连接同一个服务数据库。公开入口使用 HTTPS；非回环 `GL_WEB_ENDPOINT` 必须使用 HTTPS，证书验证保持启用，私有 CA 可通过 `NODE_EXTRA_CA_CERTS` 信任。Cookie 仅保存加密的随机 Session ID，不保存密码或 API Key；Session 摘要和有效期由数据库统一管理；Cookie 最长保留 8 小时，服务端空闲或绝对期限可更早终止会话。退出登录、改密、禁用和删除立即撤销相关会话，API Key 来源会话也受源密钥状态和有效期约束。

### 容器运行

容器构建使用校验和锁定的 Node／npm 工具链，并按原 TypeScript 发布提交重建编译器的 Go 运行时；项目 TypeScript 版本保持 7.0.2。安装依赖后、执行编译前完成版本校验和二进制替换。开发依赖及编译工具保留在单独扫描的构建阶段，只有 standalone 输出进入非 root 运行镜像。维护步骤和验证边界见 [发布验收](../docs/release.md)。

```sh
export GL_WEB_SESSION_SECRET="$(openssl rand -base64 48)"
docker compose up --build -d
```

根目录 Dockerfile 将控制台与业务服务打包为一个 `geoledger` Linux 镜像，支持 AMD64/ARM64，以 UID 10001 运行。控制台默认地址为 `http://localhost:3000`，内部通过 `127.0.0.1:7881` 连接服务端。统一启动器负责两个服务的退出与信号转发，健康检查同时验证后端就绪和控制台存活。Node 22 运行时与 standalone 产物一起进入镜像，构建工具保留在单独扫描的构建阶段。持久化、会话密钥与 HTTPS 配置见 [生产运行](../docs/production.md#容器部署)。

## 贡献

问题反馈和 Pull Request 流程见 [仓库贡献说明](../README.md#贡献)；修改 Web 后完成本页检查和相关浏览器验证。

| 路径 | 职责 |
|---|---|
| `src/app/api` | 登录、退出与认证后的浏览器接口 |
| `src/lib/operations.ts` | 已验证的请求到 TS SDK 业务方法的映射 |
| `src/components` | 页面交互，通过 HTTP 调用业务接口 |
| `src/components/map-view.tsx` | MapLibre 地图画布、地图控件、底图切换与要素高亮 |
| `src/lib/map.ts` | 图层模型：几何分类、范围、属性展示、搜索与分页（纯函数，有单元测试） |
| `src/components/ui` | 可维护、可定制的 shadcn/ui 源组件（页面控件统一使用这些组件） |
| `src/app/globals.css` | 主题、布局与响应式样式 |

版本号在浏览器请求中用十进制字符串表达，服务端转换为 bigint。GeoJSON 使用原始文本和 lossless-json，保留大整数精度；数据集要素在 MapLibre GL JS 地图工作区中按需加载与渲染（WGS 84），默认使用本地纯色底图，可离线使用；认证会话和发布恢复行为见 [控制台教程](../docs/console.md#发布结果确认与恢复)。

项目采用 [MIT 许可证](../LICENSE)，字体与组件许可见 [第三方说明](THIRD_PARTY_NOTICES.md)。

## 界面设计

控制台采用管理后台布局：深色侧边导航展示项目上下文、数据管理与系统管理分组、服务状态和当前会话；顶部提供面包屑与连接状态，内容区使用统计卡片、项目上下文卡片和表格面板组织操作。

数据集要素页采用 GIS 工作台布局：顶部工具栏放置数据来源、工作区与编辑操作；左侧“图层与要素”面板含图层树（按点、线、面分类显示与图例）、搜索和要素列表；中间为占满内容区的地图；右侧“要素信息”面板显示所选要素的属性、几何信息和 GeoJSON 原文，未选择时显示图层信息；底部属性表可开关，可拖动或用键盘调整高度。地图、列表和属性表的选中与悬停相互联动。地图控件包括放大、缩小、缩放至图层、全屏、底图切换（浅色、深色，及配置后的在线地图）、比例尺和光标经纬度。平板与手机上，左右面板改为抽屉（Sheet），要素信息在手机上以底部面板展示。

界面以中性灰白为底色，GeoLedger 橙色为强调色，文字与实心按钮使用深橙 `#cc3a05`。表格提供清晰的行分隔、悬停反馈和操作菜单；手机布局采用抽屉导航和自适应工具栏。

Inter 与 Newsreader 字体本地打包，中文衬线采用系统字体。[GeoLedger SVG 标志](public/logo.svg) 用于登录页、导航和浏览器图标，深色背景中主体跟随文字颜色。相关主题实现见 `src/app/globals.css`，组件和字体许可见 [第三方说明](THIRD_PARTY_NOTICES.md)。

按数据集变更统计的浏览器验证使用 `scripts/test-console-changes.py`，参数与上传专项验证相同，覆盖多数据集、跨分页统计、工作区版本图表头、工作区表格和版本历史。

数据类型和资源删除回归：在一次性服务环境运行 `scripts/test-console-lifecycle.py --url http://localhost:13001 --api-key-file <临时凭据文件> --chromium <浏览器路径> --geojson <EPSG:3857 的 15 要素行政边界文件>`。脚本通过真实界面验证原坐标保留和地图预览转换、点线面限制、重命名、删除和共享版本保留；不要对生产数据运行。

控件交互回归：`python3 scripts/test-console-shadcn.py --url http://localhost:13001 --api-key-file <临时凭据文件> --password-file <临时密码文件> --member-id <现有用户UUID> --chromium <浏览器路径>`。仅用于一次性测试服务，覆盖 Select 键盘操作及表单提交、Tabs 焦点与面板切换、Popover 关闭后的焦点恢复、冲突列表 Button 和手机布局。

交互质量回归：`python3 scripts/test-console-ux.py --url http://localhost:13001 --api-key-file <临时凭据文件> --member-id <无密钥用户UUID> --chromium <浏览器路径> --output target/console-ux`。仅用于一次性测试服务，验证桌面/手机弹窗焦点、未保存编辑取消与放弃、成功保存、空列表、地图标题和手机密钥布局。可追加 `--axe <axe-core/axe.min.js 路径>` 执行完整页面无障碍扫描。

实际大文件验证：`python3 scripts/test-console-large-file.py --url http://localhost:13001 --api-key-file <临时凭据文件> --chromium <浏览器路径> --geojson <GeoJSON 文件路径> --report /tmp/geoledger-large-file-report.json`。仅用于一次性测试服务；会创建项目及面数据集、通过浏览器导入整份文件、逐批加载全部地图要素、验证搜索与选择、发布并分页查询，逐个比对原文件的几何与属性。报告记录文件大小、各阶段耗时、请求状态和拓扑警告数量，截图与报告放在同一位置。

大文件统计回归：`python3 scripts/test-console-summary.py --url http://localhost:13001 --api-key-file <临时凭据文件> --chromium <浏览器路径> --fixture-report /tmp/geoledger-large-file-report.json --report /tmp/geoledger-summary-report.json`。复用大文件测试创建的隔离数据，验证工作区和历史统计数量、响应体积、耗时及真实页面展示，并检查没有读取要素明细。

集合响应浏览器验证：`python3 scripts/test-console-features.py --url http://localhost:13001 --api-key-file <临时凭据文件> --chromium <浏览器路径> --fixture-report /tmp/geoledger-large-file-report.json`。使用隔离服务，覆盖真实 GeoJSON 对象、大整数精度、GET/POST 响应一致、固定版本翻页、工作区过期、bbox、鉴权、地图列表，以及沿 `next` 链接取回整份大文件。

## 生产实例与会话

控制台无进程内会话真相。共享数据库的服务实例逐请求验证会话及用户状态；BFF 多实例共享 Cookie 加密密钥。发布使用固定请求 ID 保留重试与结果确认。

## 账户管理

默认用户名密码登录，也可切换 API Key 登录。初始或管理员重置的密码要求先在“修改密码”页更新，然后才可访问业务页面。侧栏提供个人资料、修改密码、API Key；平台管理员还可分页管理用户。用户名变更不改变用户 ID 或项目成员权限。

创建、修改、撤销和删除 API Key 以及用户管理写操作要求填写操作人的当前密码。密钥仅创建成功后显示一次；关闭窗口后无法找回。撤销与删除均需确认，同一用户可持有多个密钥以完成轮换。

完整认证浏览器验证：`python3 scripts/test-console-auth.py --url http://127.0.0.1:13000 --admin-file target/auth-refactor-integration/browser-admin.json --chromium /path/to/chromium`。仅使用一次性服务；管理员 JSON 文件权限必须为 0600。验证用户名密码与 API Key 登录、首次改密、个人资料、用户与密钥管理、CSRF、会话撤销和手机页面；截图与无凭证报告写入 `target/auth-refactor-browser`。
