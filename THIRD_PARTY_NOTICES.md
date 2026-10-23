# Third-party notices

## m1ddc

The Apple Silicon DDC helper is built from [waydabber/m1ddc](https://github.com/waydabber/m1ddc), under the MIT license. Source, license and exact revision are included in `resources/vendor/m1ddc/`. The binary is built locally. Deskflow DDC patches the upstream read conversion to use 16-bit lengths and validates Get VCP response headers, feature code, status and checksum before displaying a value; `headers/ddc_reply.h` is a local addition. The upstream `LICENSE` is distributed next to the helper in packaged applications.

## Deskflow

[Deskflow](https://github.com/deskflow/deskflow) is installed separately by the user. Deskflow DDC starts its external `deskflow-core` process and generates isolated settings. It does not bundle Deskflow binaries or link to Deskflow code. Integration was checked against tag v1.27.0 (GPL-2.0-only WITH LicenseRef-OpenSSL-Exception); retain the license notices of your Deskflow installation.

## JavaScript dependencies

Electron, Vue, Vite, TypeScript, zod, selfsigned and the Lucide icon package retain their upstream licenses. `pnpm-lock.yaml` records exact dependency versions. Dependency package license files are preserved in the packaged application dependency tree; Electron also includes its license and Chromium notices. Original application code is licensed under the MIT license; see `LICENSE`. Third-party licenses are not replaced by the project license.
