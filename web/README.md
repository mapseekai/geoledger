# GeoLedger Console

<img src="public/logo.svg" alt="GeoLedger Console" width="180">

GeoLedger Console 提供空间数据的协作管理界面：项目与数据集管理、GeoJSON 编辑、工作区发布、冲突解决、版本撤销、成员权限和审计查询。浏览器使用 HTTP，Next.js 的 Node.js 服务端通过 `@geoledger/client` 连接 GeoLedger gRPC，数据与权限由业务服务统一管理。

桌面与手机操作见 [控制台教程](../docs/console.md)。[浏览器验证](../docs/development.md#控制台浏览器验证) 可生成登录页、项目页和手机布局截图，输出到 `--screenshots` 指定目录。

## 获取代码

按照 [仓库克隆说明](../README.md#获取代码) 获取完整仓库。以下命令均从仓库根目录执行；`web/` 通过本地包依赖使用 `sdk/ts`。

## 环境准备

- [Node.js 22](https://nodejs.org/en/download) 与随附 npm，和 Web Dockerfile、主 CI 保持一致。
- 已运行的 GeoLedger 服务及管理员分配的令牌，启动方式见 [快速开始](../docs/getting-started.md)。
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
| `GL_WEB_ENDPOINT` | `http://127.0.0.1:7882`，GeoLedger 的 gRPC 地址 |
| `GL_WEB_ORIGIN` | `http://localhost:3000`，与浏览器访问的协议、主机和端口一致 |
| `GL_WEB_SESSION_SECRET` | 至少 32 字符的随机密钥，保存在本地配置或秘密管理系统 |
| `GL_WEB_BASEMAP_URL` | 可选，XYZ 栅格瓦片地址模板（HTTP(S)，含 `{z}` `{x}` `{y}`），如 `https://tile.openstreetmap.org/{z}/{x}/{y}.png`；设置后地图底图增加“在线地图”，CSP 的 `img-src` 与 `connect-src` 仅追加该来源 |
| `GL_WEB_BASEMAP_ATTRIBUTION` | 可选，在线底图署名，如 `© OpenStreetMap contributors` |

### 开发服务器

保持后端运行，在另一个终端执行：

```sh
npm --prefix web run dev
```

终端显示 Ready 后，访问 `http://localhost:3000`。输入用户令牌，进入项目列表；侧栏显示存储后端与“已连接”。此命令运行带热更新的本地开发服务，TS SDK 修改后先重新构建 SDK。

使用其他端口时，同时设置 `GL_WEB_ORIGIN`。例如将其改为 `http://localhost:3001` 后执行：

```sh
npm --prefix web run dev -- --port 3001
```

浏览器始终访问配置中的完整 origin。

## 构建、测试与部署

### 检查与构建

```sh
npm --prefix web test
npm --prefix web run typecheck
npm --prefix web run format:check
npm --prefix web run build
```

生产构建输出至 `web/.next`；浏览器与真实服务联调入口见 [开发指南](../docs/development.md#控制台浏览器验证)。

GeoJSON 文件上传专项浏览器验证：`python3 scripts/test-console-upload.py --url http://localhost:3000 --token-file target/console-test/admin-credentials.json --screenshots /tmp/geoledger-upload-shots`。使用隔离测试服务，验证大于 1 MiB 的文件、多批次导入、中断重试、精确数字和手机上传入口。 添加 `--feature-bytes 12582912` 可验证单个 12 MiB 要素经 Web/BFF/gRPC 流式上传、查询及发布；这是验证用例大小，不是产品上限。

### 生产运行

完成构建并配置运行环境后执行：

```sh
npm --prefix web start
```

生产服务使用固定的 `GL_WEB_SESSION_SECRET`，多实例共享密钥和 origin 配置。公开入口使用 HTTPS，反向代理终结 TLS；Node 服务监听内网，代理配置请求体大小、连接数、登录速率和 Origin 转发。SDK 的 `https://` 地址启用 gRPC 服务端证书验证，私有 CA 通过 `NODE_EXTRA_CA_CERTS` 信任；`GL_WEB_ENDPOINT` 使用非回环主机的 `http://` 地址时需要设置 `GL_ALLOW_INSECURE_TRANSPORT=true`（仅限隔离的内部网络）。会话 Cookie 只保存随机会话 ID，凭证保存在控制台服务端内存，退出登录立即失效；空闲期限由 `GL_WEB_SESSION_IDLE_MINUTES` 设置（默认 60 分钟），控制台重启后需要重新登录，多实例部署使用会话粘滞。

### 容器运行

```sh
docker build -f web/Dockerfile -t geoledger-console .
export GL_WEB_SESSION_SECRET="$(openssl rand -base64 48)"
docker compose --profile console up --build -d
```

Compose 启动业务服务和控制台，控制台默认使用 `http://localhost:3000`。镜像以非 root 用户运行，包含 standalone 产物、SDK 和本地字体/静态资源；运行时注入会话密钥和业务服务地址。持久化与部署配置见 [生产运行](../docs/production.md#容器部署)。

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

数据类型和资源删除回归：在一次性服务环境运行 `scripts/test-console-lifecycle.py --url http://localhost:13001 --token-file <临时凭据文件> --chromium <浏览器路径> --geojson <EPSG:3857 的 15 要素行政边界文件>`。脚本通过真实界面验证坐标转换、点线面限制、重命名、删除和共享版本保留；不要对生产数据运行。

控件交互回归：`python3 scripts/test-console-shadcn.py --url http://localhost:13001 --token-file <临时凭据文件> --chromium <浏览器路径>`。仅用于一次性测试服务，覆盖 Select 键盘操作及表单提交、Tabs 焦点与面板切换、Popover 关闭后的焦点恢复、冲突列表 Button 和手机布局。

实际大文件验证：`python3 scripts/test-console-large-file.py --url http://localhost:13001 --token-file <临时凭据文件> --chromium <浏览器路径> --geojson <GeoJSON 文件路径> --report /tmp/geoledger-large-file-report.json`。仅用于一次性测试服务；会创建项目及面数据集、通过浏览器导入整份文件、逐批加载全部地图要素、验证搜索与选择、发布并分页查询，逐个比对原文件的几何与属性。报告记录文件大小、各阶段耗时、请求状态和拓扑警告数量，截图与报告放在同一位置。
