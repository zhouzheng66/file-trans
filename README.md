# File Trans · 轻传

在自己的电脑之间传输文件和文字的桌面工具。无需账号；首次双端核对配对，之后记住设备。

当前实现第一阶段：**Windows/macOS 桌面工程、局域网自动发现、手动地址连接、双端配对、加密文字传输、文件接收确认、流式传输与 SHA-256 校验**。

第一阶段只有局域网模式；跨互联网中继、模式切换、断点续传和文件夹传输尚未实现。双方需要同时打开客户端。

## 开发启动

需要 Node.js 22+、npm、Rust stable，以及平台的 Tauri 编译依赖：macOS 使用 Xcode 命令行工具；Windows 使用 Visual Studio C++ Build Tools 和 WebView2。详见 [Tauri 环境准备](https://v2.tauri.app/start/prerequisites/)。

```bash
npm install
npm run dev
```

如果项目已有 `.tooling/cargo` 工具链，脚本优先使用该工具链，否则使用系统 Rust；不会修改系统配置。

```bash
npm run check          # TypeScript 检查
npm run build          # 前端构建
npm run test:core      # 协议、文件处理和两节点 TCP/TLS 集成测试
npm run check:rust     # 包括 Tauri 的完整 Rust 编译检查
npm run lint:rust      # Rust 静态检查
npm run fmt           # Rust 格式化
npm run desktop:build # 当前平台的桌面程序和安装包
```

编译输出位于根目录 `target/`。在 macOS 构建不能替代 Windows 原生构建；未签名安装包仅适合开发验证。

## 两台电脑的使用步骤

1. 两端运行客户端，连接同一可互访的局域网；按系统提示允许本地网络和防火墙访问。
2. 在“附近的设备”点击对方。若没有自动发现，在“我的设备”右侧点 `+`，输入对方侧边栏显示的 `IP:端口`。
3. 核对两端的八位安全校验码一致，两端分别确认。不要仅根据设备名称确认。
4. 选择已配对电脑发送文字，或选择/拖入文件。文件接收方确认后开始传输。
5. 接收文件保存在下载目录的 `File Trans` 文件夹，可在窗口底部更改。重名文件自动加序号，已有文件不会覆盖。

同名、无扩展名、零字节和任意格式文件都走二进制传输。不同系统的非法文件名会进行安全处理。单文件当前协议上限为 8 TiB，实际还受磁盘与文件系统限制。

自动发现使用 IPv4 mDNS；手动连接也使用客户端当前显示的 IPv4 地址。访客 Wi-Fi、跨 VLAN、VPN 或禁止组播的网络可能需要调整网络设置。端口由系统自动分配，重启后应以界面显示值为准。

## 数据与身份

- 设备私钥存放在 macOS Keychain / Windows Credential Manager，不写入 SQLite 或前端。
- 公共设备证书、可信设备和本机收发历史存放在应用数据目录。
- 系统凭据库不可用时启动会明确失败，不自动更换身份。
- 配对完成后按证书指纹识别设备。移除信任后，对方的后续连接不能发送文字或文件。
- 文件临时数据在传输失败时清理；断线后需重新发送。应用崩溃时留下的临时文件可按验收文档检查。

可通过环境变量为本机多实例验证指定独立配置：`FILE_TRANS_DATA_DIR`、`FILE_TRANS_DOWNLOAD_DIR`、`FILE_TRANS_DEVICE_NAME`。每个实例的应用数据目录必须不同。

## 项目结构与文档

- [中文设计文档](docs/设计文档.md)：产品逻辑、身份、网络、协议、模块与演进路线。
- [第一阶段验收](docs/第一阶段验收.md)：检查命令与真实两机测试清单。
- `apps/desktop/`：React 界面与 Tauri 宿主。
- `crates/`：不依赖界面的协议、设备身份、发现和业务核心。

公网服务未在第一阶段创建，局域网传输不依赖任何云端服务。

桌面本机联调可运行 `node scripts/rust.mjs run -p file-trans-core --example local_peer`。该示例仅监听 `127.0.0.1`，自动确认临时测试配对与接收，并回传样例文字和文件；不用于真实设备部署，退出后临时身份不保留。
