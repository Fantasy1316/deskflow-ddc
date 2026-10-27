# 从源码构建

此分支构建独立的 Deskflow DDC Preview。Electron 稳定版保留在 `v0.1.11` 标签。

## 环境

- Node.js 22.13 或更新版本、pnpm。
- Rust 1.90 或更新版本，推荐当前 stable 工具链。
- macOS：Apple Silicon、macOS 14+、Xcode Command Line Tools。
- Windows：x64、Visual Studio C++ Build Tools、Windows SDK、Microsoft Edge WebView2。

```sh
pnpm install --frozen-lockfile --ignore-scripts
pnpm prepare:engine
```

`prepare:engine` 下载固定版本的 Deskflow 官方资源和对应源码，核对预置 SHA-256，再准备本机原生辅助程序。生成文件放在 `src-tauri/resources/`，下载缓存在 `.engine-cache/`，均不提交 Git。首次准备需要联网；准备完成后，引擎启动不需要下载。

运行库以 `runtime.tar.gz` 随应用分发，避免打包工具展开 Qt 框架符号链接而增大体积。首次打开 Preview 时校验并解压到独立数据目录的 `engine/` 中；后续启动校验核心程序并复用缓存。应用更新不改变相同上游引擎版本的缓存路径，避免仅因应用更新导致辅助功能授权对象改变。

支持的资源目标是 `darwin/arm64` 和 `win32/x64`。可通过 `--cache` 选择已校验的离线缓存目录。macOS 准备脚本只编译本项目提供的原生桥，不修改已经安装的 Deskflow。

## 运行与检查

```sh
pnpm dev
pnpm build:web
pnpm test
pnpm test:rust
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --test bundled_runtime -- --ignored
```

`pnpm dev:web` 仅运行 Vue 预览，地址为 `http://127.0.0.1:5174/`。硬件功能需要 Tauri 桌面应用。

`test:rust` 运行不依赖桌面框架的后端测试；完整 `cargo test` 也检查 Tauri 命令。网络测试使用本机随机端口，进程测试只结束测试自己创建的子进程。

`bundled_runtime` 单独验证已准备的官方运行库：在临时目录解压、校验、执行 `--version` 并验证缓存复用，不启动共享或操作显示器。

## 打包

```sh
pnpm prepare:engine
pnpm build:tauri
```

在目标操作系统构建。macOS 的 `.app` 和 Windows 安装包位于 `src-tauri/target/release/bundle/`。本分支的 Mac 包使用本地 ad-hoc 签名保证包内文件完整性，未作 Developer ID 签名或公证；Windows 包也未作发行签名。不要仅分发裸二进制，必须保留内置引擎、框架/DLL、原生桥和源码许可材料。

Windows 使用系统 WebView2。没有安装 WebView2 的机器，安装程序需要获取微软运行时；该部分不能视作完全离线安装。

`.github/workflows/tauri-preview.yml` 提供 macOS ARM64 / Windows x64 的原生构建与测试，只上传构建产物，不创建 GitHub Release。

旧 Electron 脚本和测试暂时保留，用于行为回归与迁移对照；新应用入口为 `src-tauri/src/main.rs`，不会把 Electron 打入 Tauri 产物。
