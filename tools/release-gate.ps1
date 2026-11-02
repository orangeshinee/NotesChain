# 发布闸门：一条命令跑完所有验证，出汇总
# 用法：pwsh -File tools/release-gate.ps1 [-SkipBrowser]
param([switch]$SkipBrowser, [int]$Port = 18099)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\release\noteschain.exe"
Set-Location $repo

$results = New-Object System.Collections.Generic.List[object]
function Record($name, $ok, $detail) {
  $results.Add([pscustomobject]@{ 检查 = $name; 结果 = $(if ($ok) { "PASS" } else { "FAIL" }); 说明 = $detail })
  if ($ok) { "  [PASS] $name  $detail" } else { "  [FAIL] $name  $detail" }
}

"=== 0. 构建 ==="
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
& cargo build --release 2>&1 | Out-Null
$buildCode = $LASTEXITCODE
Record "cargo build --release" ($buildCode -eq 0 -and (Test-Path $exe)) "exit=$buildCode"

"=== 1. 单元测试 ==="
$testOut = (& cargo test 2>&1) -join "`n"
$testCode = $LASTEXITCODE
$testLine = [regex]::Match($testOut, 'test result: (\w+)\. (\d+) passed; (\d+) failed')
Record "cargo test" ($testCode -eq 0 -and $testLine.Success -and $testLine.Groups[1].Value -eq 'ok' -and $testLine.Groups[3].Value -eq '0') $(if ($testLine.Success) { "exit=$testCode $($testLine.Groups[2].Value) passed" } else { "exit=$testCode 解析失败" })

"=== 2. clippy ==="
$clippyOut = (& cargo clippy --all-targets 2>&1) -join "`n"
$clippyCode = $LASTEXITCODE
$warnCount = ([regex]::Matches($clippyOut, '(?m)^warning: ')).Count
$errCount = ([regex]::Matches($clippyOut, '(?m)^error')).Count
Record "cargo clippy" ($clippyCode -eq 0 -and $warnCount -eq 0 -and $errCount -eq 0) "exit=$clippyCode warnings=$warnCount errors=$errCount"

"=== 3. 接口冒烟 ==="
& pwsh -NoProfile -File (Join-Path $PSScriptRoot "smoke.ps1") *> $null
Record "接口冒烟（18 项）" ($LASTEXITCODE -eq 0) "exit=$LASTEXITCODE"

"=== 4. 边界场景 ==="
& pwsh -NoProfile -File (Join-Path $PSScriptRoot "edge-check.ps1") *> $null
Record "边界场景（18 项）" ($LASTEXITCODE -eq 0) "exit=$LASTEXITCODE"

"=== 5. 平替预演 ==="
$env:PATH = "F:\Dev\Packages\nodejs;$env:PATH"
& pwsh -NoProfile -File (Join-Path $PSScriptRoot "legacy-check.ps1") *> $null
Record "平替预演（20 项）" ($LASTEXITCODE -eq 0) "exit=$LASTEXITCODE"

if (-not $SkipBrowser) {
  "=== 6. 浏览器交互 ==="
  $gate = Join-Path $repo ".demo\gate-notes"
  if (Test-Path $gate) { Remove-Item $gate -Recurse -Force }
  # 首页的「今天」跟着系统日期走，日记必须落在当天，否则跨午夜后 schedjump 会找不到可点日程
  $now = Get-Date
  $dateStr = $now.ToString('yyyy-MM-dd')
  $monthDir = '{0}\{1}' -f $now.Year, $now.ToString('MM')
  $relWin = '{0}\{1}.md' -f $monthDir, $dateStr
  $relPosix = '{0}/{1}/{2}.md' -f $now.Year, $now.ToString('MM'), $dateStr
  New-Item -ItemType Directory -Force -Path (Join-Path $gate $monthDir), (Join-Path $gate "raw"), (Join-Path $gate "assets") | Out-Null
  $diary = @'
# 2026-11-01

## 今日日程

- 09:30-10:00 晨会 @线上

## 待办

- [ ] 闸门用待办 #测试
- [ ] 层级标签用例 #测试/子项
- [x] 已完成的 ✅ 2026-11-01 09:00

## 记录

链接：[[KPI口径]]
'@
  Set-Content -LiteralPath (Join-Path $gate $relWin) -Value ($diary -replace '2026-11-01', $dateStr) -Encoding utf8 -NoNewline
  Set-Content (Join-Path $gate "raw\KPI口径.md") "# KPI 口径`n`n- [ ] 对齐定义 #测试`n" -Encoding utf8 -NoNewline
  $png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
  [IO.File]::WriteAllBytes((Join-Path $gate "assets\pic.png"), [Convert]::FromBase64String($png))

  $log = Join-Path $gate "server.log"
  $proc = Start-Process -FilePath $exe -ArgumentList "--dir", "`"$gate`"", "--port", $Port, "--links" -RedirectStandardOutput $log -RedirectStandardError (Join-Path $gate "server.err.log") -NoNewWindow -PassThru
  try {
    $token = $null
    $deadline = (Get-Date).AddSeconds(20)
    while (-not $token -and (Get-Date) -lt $deadline) {
      if (Test-Path $log) {
        $raw = Get-Content $log -Raw
        if ($raw) { $m = [regex]::Match($raw, 'token=([0-9a-f]{64})'); if ($m.Success) { $token = $m.Groups[1].Value } }
      }
      Start-Sleep -Milliseconds 150
    }
    if (-not $token) { throw "服务未就绪" }
    $hash = "#/file/" + [uri]::EscapeDataString($relPosix)
    # 逐个场景跑（而不是 all）：一次长会话里跑全部时 Edge 偶发崩溃，逐场景更稳也更易定位
    $scenarios = @('hover','todo','autosave','conflict','search','tag','preview','virtual','clicks','quickadd','schedjump','export','settings','schedadd','resize','tagtoggle','scrollkeep')
    $probeText = ''
    $probeCode = 0
    foreach ($sc in $scenarios) {
      $one = & pwsh -NoProfile -File (Join-Path $PSScriptRoot "browser-probe.ps1") -Url "http://127.0.0.1:$Port/?token=$token" -Hash $hash -NotesDir $gate -Scenario $sc 2>&1
      if ($LASTEXITCODE -ne 0) { $probeCode = $LASTEXITCODE }
      $probeText += (($one -join "`n") + "`n")
    }
    $checks = [regex]::Matches($probeText, '(\w[\w-]*) => (.*)')
    if ($probeCode -ne 0) {
      Record "浏览器探针（非零退出）" $false "probe exit=$probeCode"
    }
    if ($checks.Count -eq 0) {
      Record "浏览器探针（无断言）" $false "probe exit=$probeCode，输出无任何 '场景 => 结果'；视为失败"
    }
    foreach ($c in $checks) {
      $value = $c.Groups[2].Value.Trim()
      $name = $c.Groups[1].Value
      $ok = $value -ne '' -and $value -notmatch 'no-|JS-ERROR' -and $value -notmatch '"added":false' -and $value -notmatch '"printed":0'
      if ($name -eq 'preview') {
        $ok = $ok -and $value -match '"defaultPreview":true' -and $value -match '"editorVisible":true' -and $value -match '"previewAgain":true' -and $value -match '"headings":[1-9]' -and $value -match '"todoBoxes":[1-9]' -and $value -match '"todoIsInput":true'
      }
      if ($name -eq 'autosave') {
        $ok = $ok -and $value -match '"marker":true'
      }
      if ($name -eq 'clicks') {
        $ok = $ok -and $value -match '"wikiFound":true' -and $value -match '"afterWiki":"#/file/'
      }
      if ($name -eq 'export') {
        $ok = $ok -and $value -match '"exportButtons":0'
      }
      if ($name -eq 'settings') {
        $ok = $ok -and $value -match '"modalOpen":true' -and $value -match '"selects":3' -and $value -match '"themeApplied":true'
      }
      Record "浏览器：$($c.Groups[1].Value)" $ok $value.Substring(0, [Math]::Min(70, $value.Length))
    }

    "=== 7. Playwright e2e（可选层） ==="
    if (Get-Command playwright-cli -ErrorAction SilentlyContinue) {
      $e2eUrl = "http://127.0.0.1:$Port/?token=$token"
      $sa = & pwsh -NoProfile -File (Join-Path $PSScriptRoot "e2e-savetype.ps1") -Url ($e2eUrl + $hash) -Session gate_e2e_save 2>&1
      $saOk = $LASTEXITCODE -eq 0
      Record "e2e：保存中继续输入" $saOk (($sa -join ' ' -split "`n" | Select-Object -Last 1))
      $cb = & pwsh -NoProfile -File (Join-Path $PSScriptRoot "e2e-checkbox.ps1") -Url ($e2eUrl + $hash) -Session gate_e2e_box 2>&1
      $cbOk = $LASTEXITCODE -eq 0
      Record "e2e：勾选往返" $cbOk (($cb -join ' ' -split "`n" | Select-Object -Last 1))
    } else {
      Record "Playwright e2e" $true "SKIP：未安装 playwright-cli"
    }
  }
  finally {
    Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  }
}

""
"===== 汇总 ====="
$results | Format-Table -AutoSize | Out-String -Width 160
$failed = @($results | Where-Object { $_.结果 -eq 'FAIL' })
if ($failed.Count) {
  "未通过 $($failed.Count) 项：$($failed.检查 -join ', ')"
  exit 1
}
"全部通过（$($results.Count) 项）"
exit 0
