# ArtsPhoto 上架整改清单

本清单对应 HarmonyOS 应用市场（AppGallery）上架审核要求。代码侧整改已完成，标记“需人工”的项目必须在提交前补齐。

## 一、已完成的整改

| 项 | 整改内容 | 落点 |
| --- | --- | --- |
| 隐私政策 | 新增隐私政策与用户协议页面，含数据说明、权限说明、未成年人保护 | `entry/src/main/ets/pages/LegalPage.ets` |
| 首启同意 | 已接入 AGC 标准化隐私声明托管服务；应用内自建弹窗（PrivacyGate）已按审核要求删除，避免双重弹窗 | AGC 托管 + `pages/Index.ets` |
| 隐私入口 | 标题、空状态页、图层面板底部均可进入隐私政策/用户协议/关于 | `pages/Index.ets` |
| 撤回同意 | 同意状态由系统隐私托管统一管理，应用内撤回按钮已删除 | AGC 隐私托管 |
| 版本展示 | “关于”页读取 bundleManager 的 versionName | `LegalPage.ets` |
| 应用信息 | `vendor` 由 `example` 改为 `Kwin` | `AppScope/app.json5` |
| 代码保护 | release 混淆暂不开启：实测现有规则会破坏 NAPI 方法属性名（`a11 is not callable`）；上架不强制混淆，优先保证可用性 | `entry/build-profile.json5` |
| 日志清理 | 移除模板 `testTag`/生命周期调试日志，统一 tag 为 `ArtsPhoto` | `EntryAbility.ets` |
| 首屏文案 | 字体探测失败不再常驻“中文可能缺失”警告，避免被审核视为异常提示 | `Index.ets` |
| 启动背景 | 启动窗口背景改为与应用一致的深色 `#0a0b0d`，消除白屏闪烁 | `entry/src/main/resources/**/element/color.json` |
| 构建兼容 | 优先使用 DevEco 自带 Node 24，规避系统 Node 25 与 hvigor 原生增量清理不兼容（`options.recursive` 报错） | `scripts/build-photocraft.ps1` |
| 发布包 | 构建脚本支持 `-PackageType app` 产出 `.app`，并拦截调试签名 | `scripts/build-photocraft.ps1` |

## 一（补充）、真机验证结果（API 26 / arm64）

已在 `4GJBB25213005674`（arm64-v8a / API 26）安装 release 构建 HAP（最终配置：混淆关闭）实测：

- 首次启动显示隐私同意弹窗，文案、按钮完整居中；点“不同意”应用退出。
- 点“同意并继续”后弹窗消失；重启应用不再弹出（同意状态已持久化）。
- 点击标题进入“关于 ArtsPhoto”，隐私政策正文、用户协议、版本信息、撤回同意按钮均正常渲染。
- 撤回同意后返回主页面，隐私弹窗重新出现。
- 弹窗显示期间点击下层“打开示例图”不会触发任何操作（遮罩拦截有效）。
- 同意后打开示例图成功：`1 个图层`、`2400 × 1500 · RGB · 8 位/通道`，release 构建未破坏 NAPI 调用。
- 开启混淆时曾实测失败（`示例载入失败：a11 is not callable`），因此当前回退为关闭混淆；若后续需要混淆，必须为 `Engine.ets` 调用的全部 NAPI 方法属性名配置 `-keep-property-name` 并重新跑本组用例。

> 2026-11-14 更新：已接入 AGC 标准化隐私声明托管服务并删除应用内自建的 PrivacyGate 弹窗，上述弹窗类验证条目由系统托管弹窗替代；首启只出现一次隐私声明弹窗。

## 一（补充 2）、上架前验证轮（2026-11-14）

本轮对当前发布包完成独立验证，报告与证据：`verify-artifacts/release-check-20261114/REPORT.md`。

- **发布包离线自检 PASS**：解包 `module.json` 确认 `debug=false`、`buildMode=release`、versionCode=1000000；`hap-sign-tool verify-app` 通过（SHA-256 digest true，证书链至 Huawei CBG Root CA G2）；提取 Profile 确认 `type=release`、`app_gallery`、绑定 `com.kwin.photocraft`、有效期至 2029-10-16。
- **素材复核 PASS**：包内 `app_icon.png` 与 `docs/launch/app-icon-1024.png` sha256 逐字节相同（`698fe149…`）；横 4 竖 3 截图规格全部符合（≤0.15 MB）。
- **模拟器运行时 PASS（MateBook Pro 2in1 / API 26，release buildMode 包，卸载重装）**：V1 首启隐私弹窗、V7 弹窗遮罩拦截、V2 同意后主界面、V3 重启不再弹窗（同意持久化，整屏像素差分 max=0）、V4 关于页版本 1.0.0 与撤回按钮齐全。
- **V5 撤回流程**：本轮设备取证中断，由上节真机验证覆盖；**V6 功能冒烟**同上由真机记录覆盖。
- **真机说明**：release 签名 HAP 调试安装报 `9568320 no signature file` 属平台限制（发布证书包不可调试安装），包正确性以离线验签为准。
- **已解决的阻断项（2026-11-14 审核驳回整改）**：联系邮箱已填写真实信息（`344922772@qq.com`），开发者名称已与 AGC 提交信息一致（王伟）；整改后必须重新出包提审。

## 一（补充 3）、审核驳回整改（2026-11-14）

AppGallery 审核驳回三项问题的整改记录：

| 审核问题 | 整改动作 | 状态 |
| --- | --- | --- |
| 问题1：应用分类及主标签与实际功能不符（应为「拍摄美化 > 图像美化」） | AGC 后台将分类由「工具」改为「拍摄美化」、主标签选「图像美化」；拍摄美化类对个人开发者无额外资质要求 | AGC 后台操作，代码侧无分类字段 |
| 问题2：应用内用户协议展示的开发者名称与开发者中心提交信息不一致 | `LegalPage.ets` 的 `DEVELOPER_NAME` 改为「王伟」、`CONTACT_EMAIL` 改为 `344922772@qq.com`，同步更新 3 份协议文档 | ✅ 已完成，需重新出包 |
| 问题3：已接入隐私托管服务仍出现两个隐私弹窗 | 删除应用内自建的 `PrivacyGate` 弹窗及全部同意状态逻辑（常量、@State、读/存/拒方法、Builder、挂载点），首启仅保留系统托管弹窗 | ✅ 已完成，需重新出包 |

### 整改后发布包（2026-11-14 出包）

- **产物**：`build/outputs/release/photocraft-release-signed.app`（29,568,085 字节 / 28.20 MB），versionCode **1000001**（上次提审占用 1000000，AGC 要求严格递增）、versionName 1.0.0 不变。
- **离线自检 PASS**：解包 `module.json` 确认 `debug=false`、`buildMode=release`、`versionCode=1000001`、`bundleName=com.kwin.photocraft`；`pack.info` 确认 apiVersion compatible/target 均为 26（Release）。
- **abc 内容 8 项全 PASS**：王伟 / 344922772@qq.com 进包；`PrivacyGate`、`撤回`、`同意并继续`、`privacyConsentVersion` 零残留；协议入口与托管表述保留。
- **验签 PASS**：`hap-sign-tool verify-app` 通过（SHA-256 digest true），证书链 `Huawei CBG Root CA G2 → Developer Relations CA G2 → 王伟(2055154120909686977), Release`，发布证书有效期 2026-10-16 ~ 2029-10-16；导出 Profile 确认 `type=release`、`bundle-name=com.kwin.photocraft`。证据与脚本在 `verify-artifacts/app-check-20261114/`。

## 二、提交前必须人工完成

1. **发布证书与 Profile（已完成）**
   - 已复用 Notepad 的发布证书 `.p12 / .cer`（`keyAlias=qingjian_notepad`），写入 `build-profile.json5` 的 `release` signingConfig。
   - 新 Profile `docs/launch/artsphotoRelease.p7b` 已就位：`bundle-name=com.kwin.photocraft`、`type=release`，已配置到 `release.material.profile`。
   - 已实际构建 `build/outputs/release/photocraft-release-signed.app`，`hap-sign-tool verify-app` 通过，Profile 类型为 `release`、`app-identifier` 与 Profile 一致。
   - 仓库中的调试签名与证书绝对路径属于本机配置，**不应作为发布配置提交**。

2. **隐私政策公网地址（阻断项）**
   - 将 `docs/launch/隐私政策.md` 托管到可公开访问的 HTTPS 页面。
   - 在 AGC「应用信息 > 隐私政策」填写该 URL，并确保与 `LegalPage.ets` 内文本一致。
   - 开发者全称与联系邮箱已替换为真实信息（代码内常量 `DEVELOPER_NAME` / `CONTACT_EMAIL` 已同步修改）。

3. **应用信息（阻断项）**
   - 应用名称：`ArtsPhoto`。提交前在 AGC 检索是否重名或近似，避免“名称混淆”驳回。
   - 应用图标：已统一 AppScope/entry 前景，并产出 `docs/launch/app-icon-1024.png`、`app-icon-216.png`（与包内 `app_icon.png` sha256 相同）。如需更换美术，必须同步更新包内资源与两份 AGC 素材后重新构建。
   - 应用分类：拍摄美化 > 图像美化（2026-11-14 审核驳回后由「工具 > 图片编辑」改报，图像美化为主标签）；内容分级按实际功能申报。
   - 应用简介、功能描述与 7 张真实运行截图已完成（横屏 4 + 竖屏 3，覆盖图层、选区、滤镜、调整）。

4. **包名与版本**
   - `bundleName` 为 `com.kwin.photocraft`，必须与 AGC 创建应用时填写的包名完全一致，且不可再变更。
   - `versionCode=1000001` / `versionName=1.0.0`（2026-11-14 整改后重新出包已递增；每次提审必须大于 AGC 已上传的最大 `versionCode`）。

5. **权限与隐私标签**
   - 本应用未声明任何 `requestPermissions`，也未使用网络、相册、剪贴板、设备标识、账号、支付、广告或第三方统计 SDK。
   - AGC「数据安全」按实际情况勾选“不收集个人信息”，与隐私政策保持一致。

6. **真机自测**
   - 首次安装后仅出现一次隐私声明弹窗（系统隐私托管），不得再出现应用内自建弹窗。
   - 同意后可正常打开图片；重启应用不再弹出隐私声明。
   - 覆盖 PNG/JPEG/WebP/TIFF/PSD 打开、编辑、撤销、导出全链路，确认无崩溃。
   - 验证 API 26（HarmonyOS 7.0）真机安装启动。

## 三、发布包生成

```powershell
# 配置好发布证书后执行
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-photocraft.ps1 -PackageType app
```

产物：`build/outputs/release/photocraft-release-signed.app`（脚本会校验并打印绝对路径）。

脚本对 `.app` 自动追加 `-p buildMode=release`，并在打包前拒绝 `debugKey` 调试签名。产物自检：

```powershell
# 解包检查：debug=false、buildMode=release
python -c "import zipfile,json,io; a=zipfile.ZipFile(r'build/outputs/release/photocraft-release-signed.app'); h=zipfile.ZipFile(io.BytesIO(a.read('entry-default.hap'))); m=json.loads(h.read('module.json')); print(m['app']['debug'], m['app']['buildMode'])"
```

仅本地验证打包链路时可临时使用：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-photocraft.ps1 -PackageType app -AllowDebugSigning
```

该命令产出的 `.app` 带调试签名，**不能提交应用市场**。

## 四、上架素材（`docs/launch/`）

| 素材 | 文件 | 状态 |
| --- | --- | --- |
| 应用介绍 / 一句话简介 | `docs/launch/应用介绍与一句话简介.md` | ✅ 可直接复制到 AGC |
| 应用图标 1024 / 216 | `docs/launch/app-icon-1024.png` / `app-icon-216.png` | ✅ 与包内 `app_icon.png` sha256 相同 |
| 横屏截图 4 张 | `docs/launch/screenshots/artsphoto_pc_{1..4}_*.jpg` | ✅ 均 1920×1080，窗口最大化 |
| 竖屏截图 3 张 | `docs/launch/screenshots/artsphoto_phone_{1..3}_*.jpg` | ✅ 均 1080×1920 |
| 隐私政策 | `docs/launch/隐私政策.md` | ✅ 待托管公网 URL |
| 用户协议 | `docs/launch/用户协议.md` | ✅ |
| 上架指引 | `docs/launch/上架指引-个人开发者.md` | ✅ 含 Notepad 实际驳回经验 |

### 本轮新增整改

- **示例图替换**：原 `sample.jpg` 是一张 Photoshop 界面截图（含 Adobe 品牌），不可用于上架；替换为 `scripts/make-sample.py` 自绘风景图，无第三方品牌与肖像。
- **颜色通道修复**：`image.createPixelMap` 的 5 处调用补 `srcPixelFormat: RGBA_8888`，修复 R/B 交换导致的画布偏色（实测天空由偏红恢复为蓝紫）。
- **2in1 窗口最大化**：`EntryAbility.maximizeIf2in1` 在 2in1 设备上调用 `maximize()`，避免 PC 截图露出壁纸/任务栏。
- **图标同源**：统一 `AppScope` 与 `entry` 的 `foreground.png` / `background.png`，新增 `app_icon.png` 作为启动窗口图标；AGC 素材与包内图标字节同源（当前 sha256 `698fe1490eb08a1d…`）。
- **图标重绘（2026-11-12）**：由 2×2 方块改为“图层卡片 + 日落照片 + 画笔”，重新生成分层前景/背景、`app_icon.png`、AGC 1024/216；已重新构建并在模拟器桌面确认显示 **ArtsPhoto** 与新图标。
- **启动/桌面图标分离**：`startWindowIcon` 改为 `$media:app_icon`，删除旧的 152px `startIcon.png`。

### 仍需人工

1. 隐私政策公网 URL（阻断；开发者名称/邮箱已填写）。
2. AGC 应用名称（ArtsPhoto）、内容分级、软著（可选）；分类已按审核要求改为「拍摄美化 > 图像美化」。
3. 整改后发布包已产出（versionCode=1000001，见「一（补充 3）」出包记录）；AGC 上传该 `.app` 前确认包名/证书未被改动。
4. 若决定把包名同步改为 `com.kwin.artsphoto`，需先改工程包名并重新申请 Profile，再更新全部资料。

## 六、1.0.1 出包记录（2026-11-19）

- **产物**：`build/outputs/release/photocraft-release-signed.app`（29,650,919 B / 28.28 MB，16:10 构建）
- **版本**：versionCode **1000002** / versionName **1.0.1**（1.0.0 上架包 1000001 严格递增）；module.json debug=false、buildMode=release；pack.info bundleType=app
- **验签**：`hap-sign-tool verify-app` success，SHA-256 digest true；叶子证书 `王伟(...),Release`（2026-10-16 ~ 2029-10-16）；Profile 绑定 com.kwin.photocraft（artsphotoRelease.p7b，type=release）
- **自检**：无 requestPermissions、无网络/统计 SDK；abc 含 ArtsPhoto/王伟/344922772@qq.com/insightIntent/EntryFormAbility/NotoSansCJK；PrivacyGate 零残留
- **构建修复**：删除 `entry/libs/{arm64-v8a,x86_64}/libentry.so`（2026-11-17 为 ohosTest 复制的残留 prebuilt，与 release CMake 产物内容不同触发 `00306049 Duplicated files`；各 HAP 现均用各自 stripped_native_libs 产物）
- **报告与证据**：`verify-artifacts/release-101-20261119/REPORT.md`
- **更新内容**：服务卡片（QuickOpenCard 2×2）、insightIntent（CreateNewDocument/OpenSampleImage）、2in1 窗口最大化、C2 条带分块 + C3 只读源包围盒性能优化、B 层交互优化（B1-B7）、D4 内置中文字体、全面审计修复（documentTurn 等）、ArkTS 测试扩容 88 用例（详见上报告）
- **提审**：上传前确认包名/证书未被改动；隐私政策公网 URL 仍为人工阻断项

## 六（补充 2）、1.1.0 出包记录（2026-11-20）

- **产物**：`build/outputs/default/photocraft-default-signed.app`（30.47 MB）
- **版本**：versionCode **1000004** / versionName **1.1.0**（1.0.1 上架包 1000002 严格递增）
- **验签**：`hap-sign-tool verify-app` success
- **自检**：无 requestPermissions、6 个 .so 齐全、.abc 正常编译代码
- **更新内容**：PSD 导出（G1）、复制可见（G2）、触屏交互修复（G3）、生态卡位做深（G4）、零权限文案+隐私可视化（G5）、A1 钢笔工具 BUG 修复、D2 smart cutout 验证
- **注**：1.0.2（versionCode 1000003）已出包验签 PASS 但用户决定不上架（与 1.0.1 间隔太短），直接跳至 1.1.0

## 六（补充 3）、1.2.0 出包记录（2026-11-21）

- **产物**：`build/outputs/default/photocraft-default-signed.app`（29.06 MB）
- **版本**：versionCode **1000005** / versionName **1.2.0**（1.1.0 严格递增）
- **验签**：`hap-sign-tool verify-app` success（SHA-256 digest true）
- **自检**：无 requestPermissions、6 个 .so 齐全（arm64-v8a + x86_64 × libc++_shared/libentry/libphotocraft_core）、.abc 正常编译代码（modules.abc + widgets.abc）
- **更新内容**：
  - 批次 2.2 图层样式预设库（G7）：FX 预设保存/套用/删除，preferences 持久化
  - 批次 2.3 蒙版触屏化（G3/L7）：链接状态指示器（只读）、启用/停用+删除蒙版触控目标 height 24→28、标题行折叠切换 minHeight 48
  - 批次 2.1 近场文件流转（G5）：nearFieldShare ShareKit 系统分享面板（1.1.0 已含）
- **构建修复**：release 构建需设置 DevEco 自带 Node 24（`NODE_HOME` + PATH 前置），规避系统 Node 25 与 hvigor 不兼容（`options.recursive` 报错）
- **测试回归**：ohosTest 31/31 PASS（含 2 PSD 导出 + 2 copyVisible + 8 A1 + 3 D2 + 其他）
- **NOT_VERIFIED（模拟器限制）**：2.2 预设 UI、2.3 蒙版 UI 在 Panel Scroll 溢出区，模拟器 uitest dump 不捕获；需物理设备验证

## 五、上架前自检命令

```powershell
# 1. 确认无权限声明
Select-String -Path entry\src\main\module.json5 -Pattern "requestPermissions"

# 2. 确认无网络/统计 SDK
Select-String -Path entry\src\main\ets\*.ets,entry\src\main\ets\**\*.ets -Pattern "@kit.NetworkKit|http|fetch|analytics"

# 3. 确认构建配置
Select-String -Path entry\build-profile.json5 -Pattern "abiFilters|obfuscation|enable"
```