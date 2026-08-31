# ADR 0004：原生 GPUI 桌面客户端

- 状态：Accepted
- 日期：2026-08-31
- 替代：ADR 0001 中的 Tauri + React 桌面客户端选择

桌面发行客户端改用 GPUI + gpui-component，以一套 Rust 代码实现窗口、三栏 IM 界面、主题、输入、网络任务编排与原生桌面交互，不再依赖 WebView。Axum 模块化单体、共享领域/协议 crate 和 PostgreSQL 事实来源保持不变。

客户端平台能力分别通过小型适配层实现：`tray-icon` 负责托盘，`notify-rust` 负责系统通知，`single-instance` 负责单实例，`keyring` 负责凭据，cargo-packager 负责安装包。SQLite 客户端核心独立为 `iamrust-client-core`，保存缓存、草稿、游标和持久化 outbox，并提供可选 AES-256-GCM 内容加密。

代价是原生 UI 生态与自动化能力不如浏览器成熟，Windows/Linux 的输入法、无障碍和桌面环境差异需要真实系统冒烟。旧版 React/Tauri 代码只保留为视觉参考和浏览器测试夹具，不再作为发行入口。
