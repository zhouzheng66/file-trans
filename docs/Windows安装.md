# Windows 安装与使用

## 下载安装包

本项目通过 GitHub Actions 在 Windows 环境生成 x64 安装包。安装应用不需要 Node.js、Rust 或 Visual Studio；这些工具只用于开发和构建。正式版本优先从 [GitHub Releases](https://github.com/zhouzheng66/file-trans/releases) 下载 `windows-x64-setup.exe` 及对应 `.sha256` 文件。其他平台见[安装与发布](安装与发布.md)。

1. 如果需要下载手动构建产物，登录 GitHub，打开仓库的 [Desktop Installers 构建页面](https://github.com/zhouzheng66/file-trans/actions/workflows/desktop-installers.yml)。
2. 选择最新一次成功的构建，确认提交与要安装的版本一致。
3. 在页面底部的 `Artifacts` 下载 `file-trans-windows-x64-installers`。
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
5. 配对后可以发送文字（Enter 发送、Shift + Enter 换行）、选择文件和文件夹，或从文件资源管理器直接拖入。已配对设备自动接收；窗口底部可“修改目录”，完成后可点击“打开所在位置”在文件资源管理器中定位内容。

默认接收目录为下载目录中的 `File Trans` 文件夹，可以在应用内更改。

## 当前版本范围

当前支持局域网直传与跨互联网中继，默认自动优先局域网，也可手动选择仅局域网或仅互联网。跨网使用前需在两端配置同一个已部署中继，并通过临时配对码和双端校验确认建立信任，详见[中继部署与使用](中继部署.md)。双方必须同时打开应用。第三阶段已支持文件断点续传、暂停恢复和传输中切换通道；普通文件源路径和接收临时文件须保留。第四阶段增加文件夹传输，保留子目录和空目录，同样支持暂停、续传和切换通道；两端需更新。文件夹准备后使用本机快照，需保留缓存并预留磁盘空间。离线暂存尚未实现。

此工作流生成的是 Windows x64 安装包；未单独提供 Windows ARM64 原生安装包。真实 Windows 与 macOS 两机互传仍需按[第四阶段验收](第四阶段验收.md)执行验证。

## 参考资料

- [Tauri Windows 安装程序与 WebView2 说明](https://v2.tauri.app/distribute/windows-installer/)
- [GitHub：下载工作流产物](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts)
