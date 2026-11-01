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
$testLine = [regex]::Match($testOut, 'test result: (\w+)\. (\d+) passed; (\d+) failed')
Record "cargo test" ($testLine.Success -and $testLine.Groups[1].Value -eq 'ok') $(if ($testLine.Success) { "$($testLine.Groups[2].Value) passed" } else { '解析失败' })

"=== 2. clippy ==="
$clippyOut = (& cargo clippy --all-targets 2>&1) -join "`n"
$warnCount = ([regex]::Matches($clippyOut, '(?m)^warning: ')).Count
$errCount = ([regex]::Matches($clippyOut, '(?m)^error')).Count
Record "cargo clippy" ($warnCount -eq 0 -and $errCount -eq 0) "warnings=$warnCount errors=$errCount"

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
  New-Item -ItemType Directory -Force -Path (Join-Path $gate "2026\11"), (Join-Path $gate "raw"), (Join-Path $gate "assets") | Out-Null
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
  Set-Content (Join-Path $gate "2026\11\2026-11-01.md") $diary -Encoding utf8 -NoNewline
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
    $hash = "#/file/" + [uri]::EscapeDataString("2026/11/2026-11-01.md")
    $probeOut = & pwsh -NoProfile -File (Join-Path $PSScriptRoot "browser-probe.ps1") -Url "http://127.0.0.1:$Port/?token=$token" -Hash $hash -NotesDir $gate -Scenario all 2>&1
    $probeText = $probeOut -join "`n"
    $probeCode = $LASTEXITCODE
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
        $ok = $ok -and $value -match '"defaultPreview":true' -and $value -match '"editorVisible":true' -and $value -match '"previewAgain":true' -and $value -match '"headings":[1-9]'
      }
      if ($name -eq 'export') {
        $ok = $ok -and $value -match '"printed":\s*1([^0-9]|$)' -and $value -match '"previewVisible":true'
      }
      Record "浏览器：$($c.Groups[1].Value)" $ok $value.Substring(0, [Math]::Min(70, $value.Length))
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
