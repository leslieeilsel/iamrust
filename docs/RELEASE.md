# 发布、签名与回滚

## 候选版流程

1. 将 workspace 版本更新为目标语义化版本，同步 `CHANGELOG.md` 与协议兼容说明。
2. 运行完整格式化、Clippy、测试、旧版 Web 回归、迁移恢复和 release build 门禁。
3. 本地至少验证当前平台的安装包；按 [平台矩阵](./PLATFORM_MATRIX.md) 记录所有目标系统的人工冒烟。
4. 创建并推送已经存在的 `vX.Y.Z` 标签，或在 Release workflow 中选择该标签手动运行。
5. 工作流用 cargo-packager 0.11.8 构建：
   - Linux x64：AppImage、Deb
   - Windows x64：NSIS
   - macOS Apple Silicon / Intel：App、DMG
6. 工作流生成 SHA-256 校验和与 SPDX JSON SBOM，然后创建或更新 GitHub 草稿 Release。
7. 维护者检查产物、校验和、已知问题和三平台冒烟记录后，才可手动发布草稿。

本地复现当前平台打包：

```bash
cargo install cargo-packager --version 0.11.8 --locked
cargo packager --release --packages iamrust-desktop-gpui
```

macOS 自动化环境使用 `CI=true`，让 DMG 工具跳过依赖 Finder 交互的布局步骤。

## 签名状态

仓库当前没有内置发行身份，因此默认产物是**未签名候选包**：

- Windows Authenticode 证书尚未接入。
- macOS Developer ID、hardened runtime、公证和 stapling 尚未接入。
- 自动更新尚未实现，也没有更新清单签名密钥。

外部证书不能提交到仓库或输出到构建日志。接入签名时，应在受保护的 CI 环境导入证书，并为 cargo-packager 配置明确的签名身份；只有 secrets 而没有签名配置不会自动产生已签名应用。

## 回滚

v0.1 没有后台自动更新，回滚通过重新发布上一兼容版本并让用户手动安装完成。不得删除失败发布的证据；应保留产物、校验和、SBOM、日志和已知问题说明。

数据库迁移原则上只向前追加。若应用回滚但数据库已迁移，上一版本必须仍能忽略新增字段/表；破坏性迁移必须先按迁移说明恢复快照。客户端本地数据库由启动迁移管理，发布前需要验证旧缓存升级与 outbox 不丢失。

发布说明至少包含新增、修复、安全、数据迁移、已知问题、平台限制和回滚方法。
