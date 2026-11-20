# ArtsPhoto

ArtsPhoto 的 HarmonyOS 移植版：Rust 图像核心（`native/`）+ ArkTS 界面（`entry/src/main/ets`），
通过 C ABI（`pc_*`）→ NAPI → `Index.d.ts` → `Engine.ets` → ArkUI 分层调用。

上架提交与合规事项见 `docs/RELEASE_CHECKLIST.md`；商店素材（文案、截图、图标、上架指引）见 `docs/launch/`。

## 环境

- HarmonyOS SDK 26.0.0 / hvigor
- Rust 目标 `aarch64-unknown-linux-ohos`、`x86_64-unknown-linux-ohos`

## 构建

```powershell
# 需要设置 DEVECO_SDK_HOME，并把 DevEco 的 jbr/bin 加入 PATH
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4   # 防止 release 编译 OOM

# 本地调试 HAP
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-photocraft.ps1

# 上架用 .app（release product，已配置 artsphotoRelease.p7b 发布 Profile）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-photocraft.ps1 -PackageType app
```

产物：

- HAP：`entry/build/default/outputs/default/entry-default-signed.hap`
- APP：`build/outputs/release/photocraft-release-signed.app`（上架提交使用）。
  调试签名产出的 `.app` 仅供本地验证，不能提交应用市场。

## 测试

```bash
# Rust 核心单元测试（桥接层 C ABI）
cd native && cargo test --manifest-path ohos-bridge/Cargo.toml --lib

# ArkTS 端到端测试（hypium）
hvigorw --mode module -p module=entry@ohosTest assembleHap
```