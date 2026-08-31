# Changelog

本项目遵循语义化版本和 Keep a Changelog 的分类方式。

## [Unreleased]

### Added

- 使用 GPUI + gpui-component 构建的无 WebView 原生桌面客户端。
- 账号、好友、单聊/群聊、文本消息、历史分页、草稿与常用消息操作。
- WebSocket 重连/同步、持久化 outbox、本地搜索与可选 AES-GCM 缓存加密。
- 原生通知、隐私预览、托盘、关闭到托盘、窗口恢复和单实例唤醒。
- cargo-packager 三平台候选包、SHA-256 校验和与 SPDX SBOM 发布工作流。

### Changed

- 原生 GPUI 客户端统一为 `apps/desktop` 和 `iamrust-desktop`。
- 删除被替代的 WebView 客户端以及 TSX、pnpm、Vite、Vitest 和 Playwright 工具链。

### Security

- Argon2id 密码哈希、轮换刷新令牌与复用检测。
- 系统凭据库、本地缓存加密、通知隐私过滤、上传头部/哈希验证和生产 TLS 配置校验。

### Known issues

- Windows/macOS 代码签名、公证与自动更新尚未接入。
- 语音/视频、完整富媒体、移动/Web 客户端和端到端加密不属于 v0.1。
