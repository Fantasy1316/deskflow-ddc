# 从源码构建

## 环境

- Node.js 22.13 或更新版本。
- pnpm 11。
- Apple Silicon macOS 或 Windows x64。
- macOS 编译原生程序需要 Xcode Command Line Tools。

```sh
pnpm install --frozen-lockfile
```

## 本地运行

macOS 首次运行先编译原生显示器和快捷键桥：

```sh
pnpm build:native
```

Windows 使用 PowerShell 调用系统原生 API，不需要执行此步骤。

```sh
pnpm dev
```

`pnpm dev:web` 只预览界面，不具备桌面硬件功能。两台电脑均需单独安装 Deskflow，使用方法见 [README](../README.md)。

## 检查

```sh
pnpm typecheck
pnpm format:check
pnpm test
pnpm build
```

macOS 可运行 `pnpm test:native` 验证 DDC 回包解析。网络测试会绑定本机 TCP 端口；模拟显示器和 Deskflow 的测试不能代替真实双机验证。

## 打包

```sh
pnpm pack:mac
pnpm pack:win
```

产物位于 `release/`。Windows 包为便携应用，macOS 包为应用目录。Windows EXE 资源编辑依赖打包工具链，跨平台构建可能需要额外环境；优先在目标系统构建验证。

也可以使用经过校验的 Electron 运行时 ZIP 进行目录打包：

```sh
node scripts/package-local.mjs darwin-arm64 /path/to/electron-vVERSION-darwin-arm64.zip
node scripts/package-local.mjs win32-x64 /path/to/electron-vVERSION-win32-x64.zip
```

ZIP 版本需与已安装的 Electron 依赖一致。脚本验证 SHA-256 后才解压；目录产物位于 `release/<应用版本>/<平台>/`。此脚本使用 `unzip`，macOS 还需要 Python 3 和 `codesign`；不包含发布签名、公证或自动更新。

分发时保留项目许可证、第三方声明和运行时附带的许可文件。请勿将用户配置、配对密钥、私钥或诊断日志提交到仓库。
