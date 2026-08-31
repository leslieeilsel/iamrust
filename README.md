# I Am Rust

I Am Rust 是一个以 Rust 为主、面向 Windows、macOS 与 Linux 的原生桌面即时通讯应用。界面借鉴经典桌面 IM 的三栏结构，但不使用 QQ 的商标、素材、账号体系、私有协议或逆向接口。

当前主客户端使用 **GPUI + gpui-component**，没有 WebView。服务端使用 Axum，服务端数据保存在 PostgreSQL，客户端使用支持可选内容加密的 SQLite 缓存与持久化 outbox。应用标识为 `app.iamrust.desktop`，当前版本为 `0.1.0` 预发布版。

## v0.1 已实现

- 原生登录/注册/会话恢复、修改密码、设备查看与远程撤销
- 会话、联系人、设置三栏导航，窗口位置与尺寸恢复，浅色/深色/跟随系统主题
- 单聊与群聊、好友搜索/申请/处理、创建群和常用群管理
- 文本消息、历史分页、草稿、回复、撤回、逐条转发、回应、收藏、详情、失败重试与删除
- WebSocket 心跳、断线重连、游标补偿同步、消息去重与持久化离线发送队列
- 可选 AES-GCM 本地 SQLite 内容加密、操作系统凭据库中的刷新令牌、本地消息搜索
- 原生通知、隐私预览、免打扰、系统托盘、关闭到托盘、单实例与重复启动唤醒
- Axum REST/WebSocket 服务、PostgreSQL 迁移、S3 兼容对象存储接口、管理 CLI 与基础可观测性
- cargo-packager 安装包配置、三平台 CI 构建矩阵、校验和与 SPDX SBOM 发布流程

v0.1 刻意不包含语音/视频通话、完整富媒体编辑器、移动端、Web 端、端到端加密、自动更新和正式代码签名。详见 [TODO.md](./TODO.md)。

## 快速开始

要求：Rust 1.98；完整本地服务还需要 Docker。

```bash
cp .env.example .env
docker compose up -d postgres minio minio-init mailpit
cargo run -p iamrust-server --bin iamrust-server
```

另开一个终端启动原生客户端：

```bash
cargo run -p iamrust-desktop
```

客户端默认连接 `http://127.0.0.1:3780`。也可使用 `IAMRUST_API_URL` 指向另一个开发服务。

## 构建与打包

```bash
cargo build --release -p iamrust-desktop
cargo install cargo-packager --version 0.11.8 --locked
cargo packager --release --packages iamrust-desktop
```

产物写入 `dist/`。macOS 本地自动化若遇到 Finder 忙碌，可使用 `CI=true cargo packager ...`；CI 会自动设置该环境。未配置发行证书时生成的是未签名候选包。

## 质量门禁

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --locked
```

## 项目结构

- `apps/desktop`：唯一的原生 GPUI 桌面客户端
- `crates/client-core`：加密缓存、草稿、同步游标与 outbox
- `apps/server`：Axum REST/WebSocket 服务和 `iamrust-admin` CLI
- `crates/domain`：领域模型与规则
- `crates/application`：服务端用例与权限边界
- `crates/protocol`：版本化 REST/WebSocket DTO
- `migrations`：PostgreSQL 迁移
- `docs`：架构、验收、安全、运维与发布文档

技术选择见 [docs/TECH_STACK.md](./docs/TECH_STACK.md)，测试边界见 [docs/TESTING.md](./docs/TESTING.md)，平台发布状态见 [docs/PLATFORM_MATRIX.md](./docs/PLATFORM_MATRIX.md)。

## 许可证

代码按 MIT 或 Apache-2.0 双许可证发布，任选其一。品牌 Logo 可用于本项目的构建与分发；派生产品应使用自己的名称和视觉标识。
