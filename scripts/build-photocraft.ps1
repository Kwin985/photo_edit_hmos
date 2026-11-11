#requires -Version 5.1
<#
.SYNOPSIS
    重建 PhotoCraft 的 arm64-v8a / x86_64 Rust 动态库，并调用 hvigor 打包签名 HAP。

.DESCRIPTION
    流程：
      1. 在 native/ 下分别对 aarch64-unknown-linux-ohos、x86_64-unknown-linux-ohos
         执行 cargo build --release；
      2. 把产物复制到 entry/libs/arm64-v8a、entry/libs/x86_64（保留既有目录结构）；
      3. 在工作区根执行 hvigorw assembleHap --no-daemon，日志同时输出到控制台和
         仓库根 build_hap.log。

    脚本不做任何产品源码修改；任一步失败都会抛出终止错误并使进程以非零码退出。
    日志由 Tee-Object 落盘（PowerShell 5.1 下为 UTF-16LE），脚本结束时会统一转换为
    无 BOM 的 UTF-8，便于后续 grep / findstr 检索。

.PARAMETER DevEcoHome
    DevEco Studio 安装目录，默认 D:\Program Files\Huawei\DevEco Studio。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-photocraft.ps1
#>
[CmdletBinding()]
param(
    [string]$DevEcoHome = 'D:\Program Files\Huawei\DevEco Studio'
)

# 任一步失败立即中止；原生命令的退出码在 Invoke-NativeCommand 内单独校验。
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# ---------------------------------------------------------------------------
# 路径与常量
# ---------------------------------------------------------------------------
$RepoRoot = Split-Path -Parent $PSScriptRoot
$LogPath = Join-Path $RepoRoot 'build_hap.log'
$NativeDir = Join-Path $RepoRoot 'native'
$HvigorwBat = Join-Path $DevEcoHome 'tools\hvigor\bin\hvigorw.bat'
$HapOutputDir = Join-Path $RepoRoot 'entry\build\default\outputs\default'

# Rust target 与 ABI 目录一一对应（与 entry/build-profile.json5 的 abiFilters 保持一致）。
$Targets = @(
    [pscustomobject]@{ RustTarget = 'aarch64-unknown-linux-ohos'; Abi = 'arm64-v8a' },
    [pscustomobject]@{ RustTarget = 'x86_64-unknown-linux-ohos'; Abi = 'x86_64' }
)

$OriginalLocation = (Get-Location).Path
$BuildStartedAt = Get-Date

# ---------------------------------------------------------------------------
# 辅助函数
# ---------------------------------------------------------------------------

<#
    通过 Tee-Object 执行原生命令：控制台实时输出 + 追加写入 build_hap.log。

    注意：PowerShell 5.1 会把重定向后的原生 stderr 包装成 ErrorRecord，在
    $ErrorActionPreference = 'Stop' 时可能提前终止管道，导致无法读取真实退出码。
    因此这里临时切到 'Continue'，命令是否成功完全由 $LASTEXITCODE 判定。
#>
function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Description
    )

    $commandLine = '> {0} {1}' -f $FilePath, ($Arguments -join ' ')
    # Tee-Object 的输出会继续流向控制台，因此这里只写一次，避免重复回显。
    $commandLine | Tee-Object -FilePath $LogPath -Append

    $previousPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $FilePath @Arguments 2>&1 | Tee-Object -FilePath $LogPath -Append
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }

    if ($null -eq $exitCode) {
        $exitCode = 0
    }
    if ($exitCode -ne 0) {
        throw ('{0} 失败，退出码 {1}。完整日志：{2}' -f $Description, $exitCode, $LogPath)
    }
}

function Write-Step {
    param([Parameter(Mandatory = $true)][string]$Message)

    $line = '=== {0}' -f $Message
    # Tee-Object 同时负责控制台输出与日志写入（-Append）。
    $line | Tee-Object -FilePath $LogPath -Append
}

<#
    解析 HarmonyOS SDK 根目录并写入 DEVECO_SDK_HOME。

    hvigor-ohos-plugin 的 sdk/lib/property-get.js 显示：HarmonyOS 工程（_isHarmonyOS=true）
    不会读取 local.properties，SDK 目录只来自环境变量 DEVECO_SDK_HOME；缺失或路径不存在时
    报 00303217 "Invalid value of 'DEVECO_SDK_HOME'"。

    因此按以下可见证据取值，不硬编码猜测：
      1. 已有且有效的 DEVECO_SDK_HOME；
      2. local.properties 中 IDE 记录的 sdk.dir（本仓库指向 DevEco 自带 sdk 目录）；
      3. <DevEcoHome>\sdk（DevEco 自带 SDK 的固定目录布局）。
#>
function Resolve-HarmonyOsSdkHome {
    param(
        [Parameter(Mandatory = $true)][string]$DevEcoHome,
        [Parameter(Mandatory = $true)][string]$RepoRoot
    )

    $candidates = New-Object System.Collections.Generic.List[string]
    if (-not [string]::IsNullOrWhiteSpace($env:DEVECO_SDK_HOME)) {
        $candidates.Add($env:DEVECO_SDK_HOME)
    }

    $localProperties = Join-Path $RepoRoot 'local.properties'
    if (Test-Path -LiteralPath $localProperties) {
        foreach ($line in Get-Content -LiteralPath $localProperties) {
            if ($line -match '^\s*sdk\.dir\s*=\s*(.+?)\s*$') {
                # Java properties 风格：双反斜杠需要还原为单反斜杠。
                $candidates.Add($matches[1].Replace('\\', '\'))
                break
            }
        }
    }

    $candidates.Add((Join-Path $DevEcoHome 'sdk'))

    foreach ($candidate in $candidates) {
        if ([string]::IsNullOrWhiteSpace($candidate)) { continue }
        if (Test-Path -LiteralPath $candidate) {
            return (Get-Item -LiteralPath $candidate).FullName
        }
    }

    throw ('未找到 HarmonyOS SDK 目录。请检查 DEVECO_SDK_HOME、local.properties 的 sdk.dir，或 DevEco 安装目录 {0}。' -f $DevEcoHome)
}

function Format-FileSize {
    param([Parameter(Mandatory = $true)][long]$Bytes)

    return ('{0:N0} 字节 ({1:N2} MB)' -f $Bytes, ($Bytes / 1MB))
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
try {
    Set-Location -Path $RepoRoot

    # 每次都生成全新的日志文件，避免上一次构建输出干扰本次判定。
    if (Test-Path -LiteralPath $LogPath) {
        Remove-Item -LiteralPath $LogPath -Force
    }

    Write-Step ('PhotoCraft 构建开始 {0}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
    ('仓库根：{0}' -f $RepoRoot) | Tee-Object -FilePath $LogPath -Append
    ('DevEco：{0}' -f $DevEcoHome) | Tee-Object -FilePath $LogPath -Append

    # --- 前置检查 -----------------------------------------------------------
    if (-not (Test-Path -LiteralPath $HvigorwBat)) {
        # 候选路径不存在时在 DevEco 安装目录内定位，找不到则明确报错，不猜测参数。
        $found = Get-ChildItem -Path (Join-Path $DevEcoHome 'tools') -Filter 'hvigorw.bat' -File -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -eq $found) {
            throw ('未找到 hvigorw.bat：{0}，且未能在 {1}\tools 下定位。' -f $HvigorwBat, $DevEcoHome)
        }
        $HvigorwBat = $found.FullName
    }

    $cargo = Get-Command -Name 'cargo' -CommandType Application -ErrorAction SilentlyContinue
    if ($null -eq $cargo) {
        throw 'PATH 中未找到 cargo，无法构建 Rust 动态库。'
    }

    # hvigorw.bat 需要 node 在 PATH 或 NODE_HOME 中（见其启动脚本）。
    # 系统 node 可用时不注入额外环境变量；不可用时给出明确指引，不臆造 SDK 变量。
    $node = Get-Command -Name 'node' -CommandType Application -ErrorAction SilentlyContinue
    if ($null -eq $node -and [string]::IsNullOrWhiteSpace($env:NODE_HOME)) {
        $bundledNode = Join-Path $DevEcoHome 'tools\node'
        if (Test-Path -LiteralPath (Join-Path $bundledNode 'node.exe')) {
            $env:NODE_HOME = $bundledNode
            ('未在 PATH 中找到 node，使用 DevEco 自带 Node：{0}' -f $bundledNode) | Tee-Object -FilePath $LogPath -Append
        }
        else {
            throw '未找到 node。请把 Node.js 加入 PATH 或设置 NODE_HOME 后重试。'
        }
    }

    # hvigor 的 PackageHap / SignHap 阶段通过 PATH 调用 java：旧成功日志 build_hap3.log
    # 中的 "Picked up JAVA_TOOL_OPTIONS: ..." 就是 java 启动时的输出。本机当前 shell
    # 既没有 java 在 PATH、也没有 JAVA_HOME，会报 00308018 "spawn java ENOENT"。
    # 此时回落到 DevEco 自带 JBR（<DevEcoHome>\jbr），并把其 bin 前置到 PATH。
    $javaCommand = Get-Command -Name 'java' -CommandType Application -ErrorAction SilentlyContinue
    if ($null -eq $javaCommand) {
        $javaHomeValid = (-not [string]::IsNullOrWhiteSpace($env:JAVA_HOME)) -and
            (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME 'bin\java.exe'))
        if ($javaHomeValid) {
            $env:PATH = (Join-Path $env:JAVA_HOME 'bin') + ';' + $env:PATH
            ('PATH 中无 java，使用 JAVA_HOME：{0}' -f $env:JAVA_HOME) | Tee-Object -FilePath $LogPath -Append
        }
        else {
            $jbrHome = Join-Path $DevEcoHome 'jbr'
            if (-not (Test-Path -LiteralPath (Join-Path $jbrHome 'bin\java.exe'))) {
                throw '未找到可用的 java。请安装 JDK/配置 JAVA_HOME，或检查 DevEco 自带 jbr 目录。'
            }
            $env:JAVA_HOME = $jbrHome
            $env:PATH = (Join-Path $jbrHome 'bin') + ';' + $env:PATH
            ('PATH 中无 java，使用 DevEco 自带 JBR：{0}' -f $jbrHome) | Tee-Object -FilePath $LogPath -Append
        }
    }

    # HarmonyOS 工程的 hvigor 只认 DEVECO_SDK_HOME（不读 local.properties 的 sdk.dir）。
    $SdkHome = Resolve-HarmonyOsSdkHome -DevEcoHome $DevEcoHome -RepoRoot $RepoRoot
    $env:DEVECO_SDK_HOME = $SdkHome


    ('cargo：{0}' -f $cargo.Source) | Tee-Object -FilePath $LogPath -Append
    ('hvigorw：{0}' -f $HvigorwBat) | Tee-Object -FilePath $LogPath -Append
    ('DEVECO_SDK_HOME：{0}' -f $SdkHome) | Tee-Object -FilePath $LogPath -Append
    ('java：{0}' -f (Get-Command -Name 'java' -CommandType Application).Source) | Tee-Object -FilePath $LogPath -Append

    # --- Step 1/3：双架构 cargo build --release ------------------------------
    Write-Step 'Step 1/3 构建 Rust 动态库（arm64-v8a / x86_64）'

    Push-Location -Path $NativeDir
    try {
        foreach ($target in $Targets) {
            $description = ('cargo build --release --target {0}' -f $target.RustTarget)
            Invoke-NativeCommand -FilePath $cargo.Source `
                -Arguments @('build', '--release', '--target', $target.RustTarget) `
                -Description $description
        }
    }
    finally {
        Pop-Location
    }

    # --- Step 2/3：复制 .so 到 entry/libs ------------------------------------
    Write-Step 'Step 2/3 复制产物到 entry/libs'

    foreach ($target in $Targets) {
        $artifact = Join-Path $NativeDir ('target\{0}\release\libphotocraft_core.so' -f $target.RustTarget)
        if (-not (Test-Path -LiteralPath $artifact)) {
            throw ('未找到 Rust 产物：{0}' -f $artifact)
        }

        $destDir = Join-Path $RepoRoot ('entry\libs\{0}' -f $target.Abi)
        # 只创建缺失目录，不删除既有 libs 结构。
        if (-not (Test-Path -LiteralPath $destDir)) {
            New-Item -ItemType Directory -Path $destDir -Force | Out-Null
        }
        $destFile = Join-Path $destDir 'libphotocraft_core.so'
        Copy-Item -LiteralPath $artifact -Destination $destFile -Force

        $destInfo = Get-Item -LiteralPath $destFile
        if ($destInfo.Length -le 0) {
            throw ('复制后的动态库为空：{0}' -f $destFile)
        }
        $artifactInfo = Get-Item -LiteralPath $artifact
        $line = ('{0} -> {1} ({2})' -f $target.RustTarget, $destFile, (Format-FileSize $destInfo.Length))
        $line | Tee-Object -FilePath $LogPath -Append

        if ($artifactInfo.Length -ne $destInfo.Length) {
            throw ('复制前后大小不一致：{0} ({1}) vs {2} ({3})' -f $artifact, $artifactInfo.Length, $destFile, $destInfo.Length)
        }
    }

    # --- Step 3/3：hvigorw assembleHap --------------------------------------
    Write-Step 'Step 3/3 hvigorw assembleHap'

    # --no-daemon 由 hvigorw --help 明确支持；任务名沿用 .hvigor 上次构建记录 assembleHap。
    Invoke-NativeCommand -FilePath $HvigorwBat `
        -Arguments @('assembleHap', '--no-daemon') `
        -Description 'hvigorw assembleHap'

    # --- 结果校验 -----------------------------------------------------------
    Write-Step '结果校验'

    $logLines = @(Get-Content -LiteralPath $LogPath)
    $successLines = @($logLines | Select-String -Pattern 'BUILD SUCCESSFUL')
    if ($successLines.Count -eq 0) {
        throw ('hvigor 未输出 BUILD SUCCESSFUL，构建失败。完整日志：{0}' -f $LogPath)
    }

    $finishedLines = @($logLines | Select-String -Pattern 'Finished release')
    $arktsErrorLines = @($logLines | Select-String -Pattern 'ArkTS:\s*(ERROR|Error)')
    # hvigor 的彩色输出会在 "ERROR" 前插入 ANSI 转义序列，因此直接按关键字匹配。
    $errorLines = @($logLines | Select-String -Pattern 'ERROR' -CaseSensitive)

    $signedHaps = @(Get-ChildItem -Path $HapOutputDir -Filter '*-signed.hap' -File -ErrorAction SilentlyContinue |
        Sort-Object -Property LastWriteTime -Descending)
    if ($signedHaps.Count -eq 0) {
        throw ('未找到签名 HAP，目录：{0}' -f $HapOutputDir)
    }
    $signedHap = $signedHaps[0]

    # --- 汇总输出 -----------------------------------------------------------
    Write-Step ('构建成功 {0}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
    ('日志文件：{0}' -f $LogPath) | Tee-Object -FilePath $LogPath -Append
    ('BUILD SUCCESSFUL 行：{0}' -f $successLines[0].Line.Trim()) | Tee-Object -FilePath $LogPath -Append
    foreach ($line in $finishedLines) {
        ('cargo {0}' -f $line.Line.Trim()) | Tee-Object -FilePath $LogPath -Append
    }
    ('ArkTS error 行数：{0}' -f $arktsErrorLines.Count) | Tee-Object -FilePath $LogPath -Append
    ('ERROR 行数：{0}' -f $errorLines.Count) | Tee-Object -FilePath $LogPath -Append
    ('签名 HAP：{0} ({1})' -f $signedHap.FullName, (Format-FileSize $signedHap.Length)) | Tee-Object -FilePath $LogPath -Append

    $elapsedSeconds = ((Get-Date) - $BuildStartedAt).TotalSeconds

    Write-Host ''
    Write-Host ('HAP: {0}' -f $signedHap.FullName) -ForegroundColor Yellow
    Write-Host ('HAP 大小: {0}' -f (Format-FileSize $signedHap.Length)) -ForegroundColor Yellow
    Write-Host ('构建耗时: {0:N1} s' -f $elapsedSeconds) -ForegroundColor Yellow

    if ($arktsErrorLines.Count -gt 0 -or $errorLines.Count -gt 0) {
        Write-Warning ('日志中存在 error 级输出（ArkTS error {0} 行 / ERROR {1} 行），请检查 {2}' -f $arktsErrorLines.Count, $errorLines.Count, $LogPath)
    }
}
finally {
    Set-Location -Path $OriginalLocation

    # PowerShell 5.1 的 Tee-Object 以 UTF-16LE 追加写入；统一转成无 BOM 的 UTF-8，
    # 便于 findstr / grep 以及人工查看。失败路径同样执行，保证日志被保留。
    if (Test-Path -LiteralPath $LogPath) {
        try {
            $rawLines = @(Get-Content -LiteralPath $LogPath)
            if ($rawLines.Count -gt 0) {
                [System.IO.File]::WriteAllLines($LogPath, [string[]]$rawLines, (New-Object System.Text.UTF8Encoding($false)))
            }
        }
        catch {
            Write-Warning ('日志编码转换失败：{0}' -f $_.Exception.Message)
        }
    }
}