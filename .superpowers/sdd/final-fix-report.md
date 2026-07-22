# Final Fix Report

## 基线与范围

- 修复基线：`158306cfcf99c74d7a32ee6807c23dd97e564033`
- 未修改 Rust、TypeScript、Cargo、package 或 `.gitignore`。
- 未执行真实 GitHub release；release API 行为均由本地 fake `gh` dry-run 覆盖。

## Findings 修复映射

### Important 1 / Important 2：release finalization 顺序与 body 安全

- `.github/workflows/release-assets.yml` 的 `release-notes` 现在显式 `needs: [windows-installer, macos-dmg]`。
- `scripts/release/finalize-release.sh` 在 notes 写入前最多轮询 30 次、每次 10 秒，要求目标 release 同时存在至少一个 `.pkg.tar.zst` 和一个 `.deb`；超时明确失败并打印最后看到的资产。
- Linux 资产齐全后重新读取当前 release body；marker 不存在时，将 notes 追加到完整当前 body，并通过 `gh release edit --notes-file` 写回，因此保留手写内容。marker 已存在时不调用 edit，rerun 幂等。
- 移除了 notes job 对 `softprops/action-gh-release` 的 body 更新，避免 action 使用旧 body 覆盖并发产生的 notes。
- `latest-json` 现在只 `needs: release-notes`。其传递依赖保证 Windows/macOS 已上传，轮询屏障保证 Arch/DEB 已上传，且 notes 已落入 release body；之后没有资产 job 再写 body。

### Important 3：Debian ABI baseline

- Linux `build-binaries` runner 固定为 `ubuntu-22.04`，不再使用浮动 `ubuntu-latest`。Jammy updates/security 的 universe 仓库提供 `libwebkit2gtk-4.1-dev`，无需退回 latest 或切换到更高 ABI 基线。
- 新增 `scripts/installer/debian/verify-amd64-elf.sh`：逐一通过 `file` 断言两个产物为 `ELF 64-bit`、`x86-64`；使用 `objdump -T` 提取全部 `GLIBC_X.Y` 符号版本、`sort -Vu` 求最高值，确保不超过 `GLIBC_2.35`。
- build job 在上传共享二进制 artifact 前执行完整 ELF/GLIBC 校验；Debian job 下载 artifact 后再次执行 x86-64 ELF 校验。

### Minor

- `Maintainer` 修复为合法格式 `zcj123v <noreply@github.com>`。
- Cargo 正式版本保持不变；首个 prerelease `-` 转换为 Debian `~`，例如 `1.2.42-beta.1` → `1.2.42~beta.1`。control `Version` 与 `.deb` 文件名统一使用转换后的版本。
- `--binaries-dir` 现在明确拒绝不存在目录、缺失二进制和非 x86-64 ELF 输入；两个输入逐一检查。
- LICENSE 未安装到 `/usr/share/doc/codexplusplus/copyright`：按 finding 要求记录不修，因为会破坏与既有 PKGBUILD payload 的完全对齐，修改 Arch payload 超出已批准范围。
- 图标仍使用仓库和既有 PKGBUILD 对齐的 `icon.png`（实际安装路径仍为 256x256）；不改变 payload，按 finding 记录不修。

## 验证命令与输出

### Shell、YAML 与 diff

```bash
bash -n scripts/release/finalize-release.sh \
  scripts/installer/debian/verify-amd64-elf.sh \
  scripts/installer/debian/cargo-version-to-deb.sh \
  scripts/installer/debian/build-package.sh
npx --yes js-yaml .github/workflows/release-assets.yml >/dev/null
npx --yes js-yaml .github/workflows/arch-package.yml >/dev/null
git diff --check
```

输出：

```text
bash syntax: ok
YAML parse: ok
git diff --check: ok
```

### Release asset polling 与 marker dry-run

使用 shell function fake `gh`，覆盖先缺 `.deb` 后资产齐全、marker absent、marker present、持续缺 `.deb` 超时四种状态；轮询间隔覆写为 0 秒。

关键输出：

```text
Waiting for Linux release assets (attempt 1/3; arch=true, deb=false)...
Release assets ready on attempt 2/3.
Fork notes appended while preserving the existing release body.
Release assets ready on attempt 1/3.
Fork notes marker already exists; release body is unchanged.
release finalization dry-runs: success, marker-present, timeout all passed
error: timed out waiting for both a .pkg.tar.zst and a .deb release asset after 2 attempts
Last observed release assets:
codexplusplus.pkg.tar.zst
```

断言：marker absent 时保留 `Handwritten release body`、marker 恰好一个、edit 恰好一次；marker present 时 edit 次数为零；超时退出非零且错误文本明确。

### 版本转换、metadata 与输出文件名

直接转换测试：

```bash
scripts/installer/debian/cargo-version-to-deb.sh 1.2.41
scripts/installer/debian/cargo-version-to-deb.sh 1.2.42-beta.1
```

输出：

```text
formal=1.2.41
prerelease=1.2.42~beta.1
```

由于当前本机缺少 `dpkg-deb`，使用只模拟 `--build/--info/--contents` 合约的临时 stub，配合真实 `/usr/bin/true` x86-64 ELF，运行完整 `build-package.sh` staging/metadata/文件名路径。关键输出：

```text
formal package: codexplusplus_1.2.41_amd64.deb
prerelease package: codexplusplus_1.2.42~beta.1_amd64.deb
metadata: Version: 1.2.42~beta.1 | Architecture: amd64 | Maintainer: zcj123v <noreply@github.com>
stubbed package integration and negative arguments: passed
```

### 参数与二进制负向

执行并断言退出码：

```text
negative exit 2: build-package.sh --binaries-dir
negative exit 2: build-package.sh --unknown
negative exit 2: build-package.sh one two
negative exit 1: build-package.sh --binaries-dir <missing-dir> <out>
negative exit 1: build-package.sh --binaries-dir <one-binary-only> <out>
negative exit 1: build-package.sh --binaries-dir <fake-text-binary-dir> <out>
```

### ELF / GLIBC 校验

以本机真实 x86-64 `/usr/bin/true` 覆盖通过与 ceiling 失败，并以文本文件覆盖架构/ELF 失败：

```text
verified x86-64 ELF: <tmp>/ok
verified GLIBC ceiling: <tmp>/ok requires at most GLIBC_2.34 (allowed GLIBC_2.35)
GLIBC ceiling failure exit=1: error: <tmp>/ok requires GLIBC_2.34, newer than allowed GLIBC_2.33
architecture failure exit=1: error: expected an x86-64 ELF binary: <tmp>/fake (ASCII text)
```

## 限制

- 未触发真实 `release.published`，符合测试约束。
- 当前机器没有 `dpkg-deb`，所以 archive 构建用临时严格 stub 验证脚本控制流、metadata 和文件名；GitHub `ubuntu-22.04` job 会使用真实 `dpkg-deb` 完成包构建与既有 archive 自检。

## Follow-up final fix（基线 `3d07a9c`）

### Important 1：用 reusable workflow 建立真实依赖

- `.github/workflows/arch-package.yml` 新增 `workflow_call`，声明默认 `false` 的 boolean input `attach_to_release`；删除直接 `release.published` trigger，保留 push、PR 与无上传选项的 `workflow_dispatch`。
- Arch 与 Debian 的 release upload 只在 `inputs.attach_to_release` 为 true 时运行，并显式使用 caller release 的 `${{ github.event.release.tag_name }}`。push、PR、manual dispatch 都使用默认 false，不会上传 release。
- `.github/workflows/release-assets.yml` 新增 `linux-packages` reusable workflow job，传 `attach_to_release: true`、`secrets: inherit` 与 `permissions: contents: write`。
- `release-notes` 现在严格 `needs: [windows-installer, macos-dmg, linux-packages]`；三个平台的全部 `softprops` asset writer 成功结束后才读取并编辑 body。
- `scripts/release/finalize-release.sh` 删除全部扩展名 polling、timeout 和 sleep 逻辑，只负责读取当前 body、marker 幂等判断、保留完整手写 body 并 `gh release edit --notes-file`。
- `latest-json` 继续只依赖 `release-notes`，通过传递依赖读取 notes 后且所有 assets 已完成的 release。

### Important 2：拒绝所有无法证明兼容的 GLIBC version need

- `verify-amd64-elf.sh` 改用 `readelf --version-info --wide`，读取 `.gnu.version_r` 中的全部 version needs，而不是只看动态符号表。
- 提取所有 `GLIBC_[A-Za-z0-9_.]+`；任何不匹配 `GLIBC_<纯数字点版本>` 的需求（包括 `GLIBC_ABI_DT_RELR`）立即明确失败。纯数字版本继续以 `sort -V` 比较 `GLIBC_2.35` ceiling。
- `build-package.sh` 调 helper 时也传入 `--max-glibc GLIBC_2.35`，因此本地/CI `--binaries-dir` 打包路径不能绕过 ABI ceiling。
- Reviewer 提到的手写其他 libc version namespace 与 executable subtype 细化记录为 Minor，未扩大本次指定范围；现有逻辑仍严格要求 x86-64 ELF，并拒绝所有检测到的非 numeric GLIBC needs。

### Follow-up 验证

Workflow YAML 与 reusable 结构：

```bash
npx --yes js-yaml .github/workflows/arch-package.yml
npx --yes js-yaml .github/workflows/release-assets.yml
# 用 Node 对解析后的 JSON 断言 workflow_call、无 release trigger、input 默认 false、
# 两个 upload guard/tag、caller permissions/secrets、release-notes needs 与 latest-json needs。
```

输出：

```text
workflow_call triggers, guard, tag, permissions and needs: ok
YAML parse: ok
```

全部相关 shell 与静态检查：

```bash
bash -n scripts/release/finalize-release.sh \
  scripts/installer/debian/verify-amd64-elf.sh \
  scripts/installer/debian/cargo-version-to-deb.sh \
  scripts/installer/debian/build-package.sh
git diff --check
```

输出：

```text
bash syntax: ok
git diff --check: ok
```

Finalize fake `gh` marker absent/present：

```text
Fork notes appended while preserving the existing release body.
Fork notes marker already exists; release body is unchanged.
finalize marker absent/present: passed
```

断言 absent 时完整保留 `Handwritten release body`、marker 恰好一个、edit 一次；present 时 edit 零次。脚本已无 asset query/poll/sleep。

GLIBC 模拟 numeric pass / above ceiling / non-numeric：

```text
verified GLIBC ceiling: <tmp>/binary requires at most GLIBC_2.35 (allowed GLIBC_2.35)
numeric above exit=1: error: <tmp>/binary requires GLIBC_2.36, newer than allowed GLIBC_2.35
non-numeric exit=1: error: <tmp>/binary requires unsupported non-numeric GLIBC version GLIBC_ABI_DT_RELR; cannot prove compatibility with GLIBC_2.35
```

真实 DT_RELR 回归：

```bash
cc main.c -Wl,-z,pack-relative-relocs -o relr
readelf --version-info --wide relr | grep GLIBC_ABI_DT_RELR
scripts/installer/debian/verify-amd64-elf.sh --max-glibc GLIBC_2.35 relr
```

输出与断言：

```text
Name: GLIBC_ABI_DT_RELR
real DT_RELR binary exit=1: error: relr requires unsupported non-numeric GLIBC version GLIBC_ABI_DT_RELR; cannot prove compatibility with GLIBC_2.35
```

`build-package.sh` ceiling 传递回归（fake `file`/`readelf` 报 GLIBC_2.36）：

```text
build-package GLIBC ceiling exit=1: error: <tmp>/bin/codex-plus-plus requires GLIBC_2.36, newer than allowed GLIBC_2.35
```

版本回归：

```text
formal=1.2.41
prerelease=1.2.42~beta.1
```

仍未执行真实 release。
