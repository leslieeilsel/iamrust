# 测试策略

## 自动化层级

- Rust 单元测试：领域规则、协议序列化、认证轮换、权限、消息幂等、通知隐私文案与桌面辅助逻辑
- Rust 集成测试：Axum REST/WebSocket、同步游标、离线 outbox、SQLite 缓存迁移、AES-GCM 加解密和消息搜索
- 原生客户端门禁：`iamrust-client-core` 与 `iamrust-desktop` 的测试、全 target Clippy `-D warnings`、真实 GPUI 启动冒烟
- 服务与数据门禁：从空库迁移、上一版本升级、备份和恢复演练

## CI

`ci.yml` 执行格式、Clippy、Rust 测试、依赖策略、迁移恢复以及 Linux、Windows、macOS 的 GPUI release 构建。`release.yml` 使用 cargo-packager 生成候选安装包、SHA-256 校验和与 SPDX SBOM。

CI 编译成功只证明源码与打包配置可构建。托盘、通知、系统凭据库、窗口恢复、输入法和安装/卸载行为仍须按 [平台矩阵](./PLATFORM_MATRIX.md) 在真实系统人工冒烟后，候选版才能提升为稳定版。

## 本地命令

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release --locked -p iamrust-desktop
```

## 原生冒烟清单

1. 首次启动、注册/登录、退出和凭据恢复。
2. 三栏切换、输入法、消息发送、历史分页、断网重试和重启恢复。
3. 通知开关、隐私预览、免打扰、前台抑制和自身消息抑制。
4. 托盘显示/退出、关闭到托盘、重复启动唤醒和窗口状态恢复。
5. 安装、覆盖安装、卸载以及本地数据保留策略。

## 性能基线

单节点开发目标为 1,000 个活跃 WebSocket、文本消息 ACK P95 小于 500 ms（同区域）、同步页 200 个事件 P95 小于 1 秒。`scripts/load-test.mjs` 会创建两个测试账号、好友关系和单聊，并发发送唯一消息，再通过接收方同步游标检查丢失与重复；禁止指向生产用户数据。

2026-08-30 本机内存存储基线（Apple Silicon，5 并发 × 20 条）：100/100 ACK 与同步成功，P50 2 ms、P95 3 ms、P99 3 ms。该结果只用于回归参照，不代表生产容量承诺。
