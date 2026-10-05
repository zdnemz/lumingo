# Environment record

Exact versions of what this repository has been built and tested with. A row is
filled only from a command that was run on the machine named in the section
title. Rows marked **not recorded** have not been run on that machine yet.

## 1. Linux build container (used by the coding agent)

Recorded on 2026-10-05.

| Item | Value | Command |
|---|---|---|
| OS | Ubuntu 24.04.4 LTS, kernel 6.18.44 | `cat /etc/os-release`, `uname -sr` |
| CPU | Intel(R) Xeon(R) Processor @ 2.80GHz, 4 logical processors, 1 thread per core | `lscpu` |
| Memory | about 15 GB | `free -g` |
| Rust | 1.97.0 (2d8144b78 2026-07-07), host `x86_64-unknown-linux-gnu`, LLVM 22.1.6 | `rustc -vV` |
| Cargo | 1.97.0 (c980f4866 2026-06-30) | `cargo -V` |
| Node | v22.22.0 | `node -v` |
| npm | 10.9.4 | `npm -v` |
| pnpm | 10.28.0 | `pnpm -v` |
| Test browser | Chromium 141.0.7390.37 (headless) | `chrome --version` |

Limits of this machine. It has no audio devices and no audio development
headers, and it cannot reach Hugging Face, GitHub release downloads, or the
ONNX Runtime download host. Speech engines and model downloads are therefore
not built or run here. This machine is not a capped profile: it has four
logical processors but about 15 GB of memory, so no result from it may be
tagged `floor`.

## 2. Windows 11 development machine (owner)

Not recorded. The owner fills this table by running the commands on the
machine that produced the results.

| Item | Value | Command |
|---|---|---|
| OS build | not recorded | `winver` |
| CPU string | not recorded | `Get-CimInstance Win32_Processor \| Select-Object Name` |
| Memory | not recorded | `Get-CimInstance Win32_ComputerSystem \| Select-Object TotalPhysicalMemory` |
| Rust (MSVC toolchain) | not recorded | `rustc -vV` |
| MSVC build tools | not recorded | `vswhere -latest -property catalog_productDisplayVersion` |
| Node | not recorded | `node -v` |
| pnpm | not recorded | `pnpm -v` |
| Browsers used for testing | not recorded | each browser's About page |

## 3. Toolchain pin

`rust-toolchain.toml` pins Rust to 1.97.0 with rustfmt and clippy. The pin is
changed in its own commit, and this file is updated in the same commit.
