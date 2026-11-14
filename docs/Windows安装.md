# Windows 安装与使用

## 下载安装包

本项目通过 GitHub Actions 在 Windows 环境生成 x64 安装包。安装应用不需要 Node.js、Rust 或 Visual Studio；这些工具只用于开发和构建。

1. 登录 GitHub，打开仓库的 [Windows Installer 构建页面](https://github.com/zhouzheng66/file-trans/actions/workflows/windows-installer.yml)。
2. 选择最新一次成功的构建，确认提交与要安装的版本一致。
3. 在页面底部的 `Artifacts` 下载 `file-trans-windows-x64-installer`。
4. 解压下载的 ZIP，运行其中以 `-setup.exe` 结尾的安装程序，按向导完成安装。不要把仓库源码 ZIP 当作安装包。
5. 如果安装程序提示安装 Microsoft WebView2，允许其安装；首次补装运行环境可能需要联网。

安装包目前没有代码签名，Windows 可能显示“未知发布者”或 SmartScreen 提示。请核对下载来源为本仓库，以及构建对应的提交；确认来源后再决定是否继续安装，不需要关闭系统安全防护。

安装包旁附有 `.sha256` 文件，可以在 PowerShell 中使用 `Get-FileHash -Algorithm SHA256 "安装程序路径"`，对照文件中的摘要检查下载完整性。摘要不能代替发布者签名。

构建产物保留 30 天。若已过期，可在构建页面点击 `Run workflow` 重新生成；此操作需要仓库的相应权限。构建失败时请查看失败步骤，失败构建不会提供可用安装包。

## 首次连接另一台电脑

1. 两台电脑都打开 File Trans，连接同一可互访的局域网。
2. 系统询问防火墙或本地网络权限时，允许应用在你信任的专用网络中通信。
3. 在“附近的设备”选择另一台电脑；没有自动发现时，点击“我的设备”旁的 `+`，输入对方界面显示的 `IP:端口`。
4. 核对两端的八位安全校验码一致，然后在两端分别确认配对。
5. 配对后可以发送文字或选择、拖入任意格式文件。文件接收方确认后开始传输。

默认接收目录为下载目录中的 `File Trans` 文件夹，可以在应用内更改。

## 当前版本范围

当前仅实现局域网传输，双方必须同时打开应用。跨互联网传输、网络模式切换、断点续传和文件夹传输尚未实现。

此工作流生成的是 Windows x64 安装包；未单独提供 Windows ARM64 原生安装包。真实 Windows 与 macOS 两机互传仍需按[第一阶段验收](第一阶段验收.md)执行验证。

## 参考资料

- [Tauri Windows 安装程序与 WebView2 说明](https://v2.tauri.app/distribute/windows-installer/)
- [GitHub：下载工作流产物](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts)
