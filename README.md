# photo_edit_hmos

PhotoCraft 的 HarmonyOS 移植版：Rust 图像核心（`native/`）+ ArkTS 界面（`entry/src/main/ets`），
通过 C ABI（`pc_*`）→ NAPI → `Index.d.ts` → `Engine.ets` → ArkUI 分层调用。

## 环境

- HarmonyOS SDK 26.0.0 / hvigor
- Rust 目标 `aarch64-unknown-linux-ohos`、`x86_64-unknown-linux-ohos`

## 构建

```powershell
# 需要设置 DEVECO_SDK_HOME，并把 DevEco 的 jbr/bin 加入 PATH
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4   # 防止 release 编译 OOM
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-photocraft.ps1
```

产物：`entry/build/default/outputs/default/entry-default-signed.hap`

## 测试

```bash
# Rust 核心单元测试（桥接层 C ABI）
cd native && cargo test --manifest-path ohos-bridge/Cargo.toml --lib

# ArkTS 端到端测试（hypium）
hvigorw --mode module -p module=entry@ohosTest assembleHap
```