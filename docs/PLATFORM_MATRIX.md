# 平台支持矩阵

| 平台 | v0.1 基线 | 候选产物 | CI runner | 当前证据 |
| --- | --- | --- | --- | --- |
| Windows | Windows 10/11 x64 | NSIS `.exe` | `windows-2025` | release 构建与打包流程已配置；真实安装冒烟待发行前记录 |
| macOS | macOS 15+，Apple Silicon / Intel 分包 | `.app.zip`、`.dmg` | `macos-15`、`macos-15-intel` | Apple Silicon `.app`/DMG 已本地生成；应用启动、单实例、激活标记和 Info.plist 已验证 |
| Linux | Ubuntu 24.04 x64 及兼容环境 | AppImage、Deb | `ubuntu-24.04` | release 构建与打包流程已配置；真实桌面冒烟待发行前记录 |

Linux Deb 声明 GTK 3、AppIndicator、XDo、XKB、Fontconfig、Vulkan、Wayland 与 XCB 运行依赖。AppImage 仍需目标系统提供图形栈和通知/托盘所需的桌面服务。

## 稳定版提升条件

每个平台都必须记录以下人工结果：

1. 干净系统安装、覆盖安装、启动、卸载。
2. 登录与系统凭据库恢复。
3. 中文输入、复制粘贴、会话切换和消息发送。
4. 托盘显示/退出、关闭到托盘、第二次启动唤醒。
5. 通知权限、普通预览、隐私预览、免打扰和前台抑制。
6. 窗口尺寸/位置恢复、浅色/深色/跟随系统主题。
7. 断网、重连、离线 outbox 与应用重启恢复。

CI 证明指定 runner 可以编译或打包；它不能替代签名、公证、系统权限弹窗、桌面环境差异和卸载行为的真实验证。当前候选包默认未签名，不能标记为正式稳定发行。
