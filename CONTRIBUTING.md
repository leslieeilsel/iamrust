# 参与贡献

1. 从 `main` 创建短生命周期分支，提交信息使用 Conventional Commits。
2. 修改协议时保持向后兼容；破坏性变化必须同时更新 `docs/VERSIONING.md`。
3. 不得提交真实密钥、访问令牌、消息正文样本或用户数据。
4. 提交前运行 `pnpm format:check && pnpm lint && pnpm test && pnpm build`。
5. GPUI 变化需验证键盘、焦点、中文输入、窄窗口、深浅色、托盘和通知；旧版 Web 测试不能替代原生冒烟。

安全问题不要公开提交 Issue，请按 `SECURITY.md` 私下报告。
