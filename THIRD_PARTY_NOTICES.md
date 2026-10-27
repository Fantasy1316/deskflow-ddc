# Third-party notices

## m1ddc

The Apple Silicon DDC helper is built from [waydabber/m1ddc](https://github.com/waydabber/m1ddc), under the MIT license. Source, license and exact revision are included in `resources/vendor/m1ddc/`. The binary is built locally. Deskflow DDC patches the upstream read conversion to use 16-bit lengths and validates Get VCP response headers, feature code, status and checksum before displaying a value; `headers/ddc_reply.h` is a local addition. The upstream `LICENSE` is distributed next to the helper in packaged applications.

## Deskflow

[Deskflow](https://github.com/deskflow/deskflow) 1.27.0 is bundled as a separate, unmodified `deskflow-core` process, with its official runtime dependencies. It is not linked into the Rust application. The runtime is prepared from official release assets verified with pinned SHA-256 digests. Its corresponding source at commit `081f6478e654a112168879e67a41ce7d7e4ae3af` and license texts are shipped in `resources/deskflow/` inside each application. This project is independent of the Deskflow maintainers.

Deskflow uses GPL-2.0-only WITH LicenseRef-OpenSSL-Exception. Its Qt, OpenSSL, Microsoft runtime and other dependencies retain their respective licenses. The preparation script preserves the complete upstream runtime directory, including DLLs/frameworks and any provided notices. See the bundled source archive and `resources/deskflow/licenses/` for Deskflow notices. Original MIT application code does not replace third-party licenses.

## Rust and Tauri dependencies

Tauri 2, its plugins, Tokio, serde, mdns-sd, keyring, rcgen and cryptographic crates retain their upstream licenses. `src-tauri/Cargo.lock` records exact versions. The system WebView is WKWebView on macOS and Microsoft Edge WebView2 on Windows. WebView2 may require installation on Windows machines where it is absent.

## JavaScript dependencies

Vue, @tauri-apps/api, Vite, TypeScript, zod and the Lucide icon package retain their upstream licenses. `pnpm-lock.yaml` records exact dependency versions. The Tauri application ships compiled frontend assets and uses the system WebView; it does not ship an Electron/Chromium runtime. Original application code is licensed under the MIT license; see `LICENSE`. Third-party licenses are not replaced by the project license.
