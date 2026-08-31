# I Am Rust 技术栈与架构

状态：v0.1 只保留原生 GPUI 客户端。此前的 WebView 桌面方案已被 ADR 0004 替代，相关客户端源码和前端工具链均已移除。

## 1. 设计目标

- 桌面主程序、网络、本地存储、共享领域和服务端均使用 Rust
- Windows、macOS、Linux 共用一套原生 UI 代码，不依赖 WebView
- 首版只覆盖账号、好友、会话、文本消息、群聊和桌面集成的完整闭环
- 服务端 PostgreSQL 是事实来源；SQLite 是客户端缓存、草稿和离线工作区
- 使用模块化单体，暂不引入微服务、消息代理或 Kubernetes

## 2. 桌面客户端

### UI 与应用壳

- **GPUI 0.2 + gpui-component 0.5**：窗口、布局、组件、主题、焦点和输入
- **单一原生窗口**：登录态与主界面在同一窗口切换；主界面采用导航栏、列表栏、内容栏三栏结构
- **主题与窗口状态**：浅色、深色、跟随系统；恢复位置、尺寸与最大化状态
- **原生集成**：`tray-icon`、`notify-rust`、`single-instance` 与 `keyring`
- **打包**：cargo-packager 0.11.8，应用标识 `app.iamrust.desktop`

GPUI 的平台事件循环保持在主线程；网络、SQLite 和耗时操作由 Tokio runtime 执行，结果通过 GPUI task 回到界面实体。界面不获得任意 SQL、Shell 或文件系统执行能力。

### 客户端数据层

- **HTTP**：`reqwest` + Rustls
- **实时连接**：`tokio-tungstenite`，包含协议握手、心跳、重连与游标补偿同步
- **本地数据库**：SQLite + SQLx，保存会话、消息、草稿、同步游标、设置与 outbox
- **本地内容保护**：用户可开启 AES-256-GCM 缓存加密；密钥保存在操作系统凭据库
- **会话凭据**：访问令牌仅驻留内存，刷新令牌保存在操作系统凭据库
- **消息一致性**：客户端 UUIDv7 幂等 ID、服务端 ACK、事件 ID 去重、持久化 outbox 与失败重试

## 3. Rust 共享层

- **异步运行时**：Tokio
- **序列化**：Serde
- **ID**：实体使用 UUID；新消息使用 UUIDv7 `client_message_id`
- **时间**：服务端与协议统一使用 UTC，显示层转为本地时间
- **错误**：库边界使用 `thiserror`，应用装配使用 `anyhow`
- **领域与协议**：`iamrust-domain` 与 `iamrust-protocol` 同时被客户端和服务端复用
- **日志与追踪**：`tracing` / `tracing-subscriber`

依赖方向为：`domain` 不依赖 UI、数据库或网络；`application` 依赖 `domain`；客户端和服务端负责实现网络、持久化与平台适配器。

## 4. 服务端

- **框架**：Axum 0.8 + Tower / tower-http
- **持久化**：PostgreSQL + SQLx 迁移
- **对象存储**：S3 兼容接口，本地使用 MinIO
- **认证**：Argon2id、短期访问令牌、轮换刷新令牌与复用检测
- **协议**：REST 用于认证、好友、群组、历史和设置；WebSocket 用于 ACK、消息、同步事件与在线状态
- **接口文档**：OpenAPI / utoipa
- **观测**：结构化日志、Prometheus 指标，可选 OpenTelemetry

## 5. 基础设施与交付

- Docker Compose：PostgreSQL、MinIO、Mailpit，以及可选完整服务栈
- GitHub Actions：格式化、Clippy、测试、依赖策略、迁移恢复和三平台原生构建
- cargo-packager：Linux AppImage/Deb、Windows NSIS、macOS App/DMG
- 发布草稿：SHA-256 校验和与 SPDX JSON SBOM

## 6. MVP 消息路径

1. GPUI 编辑器创建文本与唯一 `client_message_id`。
2. 客户端先将待发送项写入 SQLite outbox，并立即显示 pending 状态。
3. WebSocket 可用时发送命令；不可用时保留待重试项。
4. 服务端在事务中校验成员权限，按用户与客户端 ID 幂等落库并追加同步事件。
5. ACK 将本地状态更新为 sent；实时事件归并到本地缓存。
6. 重连时客户端先按游标补齐缺失事件，再继续消费实时事件。

该路径提供至少一次传输上的最终一致性；“恰好一次”由业务幂等与去重组合实现，不是网络层承诺。

## 7. v0.1 暂缓能力

- 语音/视频、屏幕共享和系统音频
- 完整图片/文件编辑与传输 UI
- 多窗口、全局快捷键和自动更新
- 正式代码签名、公证与商店分发
- 移动端、Web 端、机器人/插件生态
- 端到端加密；在完成多设备密钥设计和外部审计前不得宣称支持

## 8. 仓库结构

```text
I-Am-Rust/
├── apps/
│   ├── desktop/               # 唯一的原生 GPUI 桌面客户端
│   └── server/                # Axum 模块化单体
├── crates/
│   ├── client-core/           # SQLite 缓存、加密、草稿和 outbox
│   ├── domain/                # 领域实体与规则
│   ├── protocol/              # REST/WebSocket 公共 DTO
│   ├── application/           # 服务端用例与端口
│   └── test-support/          # 测试构造器与 fixture
├── migrations/                # PostgreSQL 迁移
├── infra/                     # 容器与部署配置
└── docs/                      # 架构、安全、测试和发布文档
```

## 9. 参考资料

- [GPUI Component](https://longbridge.github.io/gpui-component/)
- [GPUI Component 安装与平台依赖](https://longbridge.github.io/gpui-component/docs/installation)
- [cargo-packager](https://docs.crabnebula.dev/packager/)
- [Axum WebSocket](https://docs.rs/axum/latest/axum/extract/ws/)
- [SQLx](https://docs.rs/sqlx/latest/sqlx/)
