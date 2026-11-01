# NotesChain 端到端冒烟测试
# 用法：pwsh -File tools/smoke.ps1 [-Exe target\release\noteschain.exe]
# 说明：在 .demo\smoke-notes 下造一份小语料，起服务后逐项验证接口与回写，最后停掉进程。

param(
  [string]$Exe = "target\release\noteschain.exe",
  [int]$Port = 18095
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = if ([IO.Path]::IsPathRooted($Exe)) { $Exe } else { Join-Path $repo $Exe }
if (-not (Test-Path $exe)) { throw "找不到可执行文件: $exe（先运行 cargo build --release）" }

$notes = Join-Path $repo ".demo\smoke-notes"
if (Test-Path $notes) { Remove-Item $notes -Recurse -Force }

# 日记必须落在「今天」，否则 /api/schedule 的当日断言会在跨天时失效
$now = Get-Date
$dateStr = $now.ToString('yyyy-MM-dd')
$monthDir = '{0}\{1}' -f $now.Year, $now.ToString('MM')
$relWin = '{0}\{1}.md' -f $monthDir, $dateStr
$relPosix = '{0}/{1}/{2}.md' -f $now.Year, $now.ToString('MM'), $dateStr

New-Item -ItemType Directory -Force -Path (Join-Path $notes $monthDir), (Join-Path $notes "raw"), (Join-Path $notes "assets") | Out-Null

$diary = @'
# 2026-11-01

## 今日日程

- 09:30-10:00 晨会 @线上
- 15:00-17:00 供应链周会 #工作/供应链
- 记得带电脑充电器

## 待办

- [ ] 发供应链周报 #工作/供应链
- [ ] 先对采购模块
  - [ ] 子项
- [x] 已完成 ✅ 2026-11-01 09:12

## 记录

```sql
SELECT 1;
- [ ] 代码块里的不算
```

细节见 [[raw/KPI口径]]。
'@
Set-Content -LiteralPath (Join-Path $notes $relWin) -Value ($diary -replace '2026-11-01', $dateStr) -Encoding utf8
Set-Content -Path (Join-Path $notes "raw\KPI口径.md") -Value "# KPI 口径`n`n- [ ] 对齐指标定义 #工作/供应链/KPI`n" -Encoding utf8

# 1x1 png
$png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
[IO.File]::WriteAllBytes((Join-Path $notes "assets\pic.png"), [Convert]::FromBase64String($png))

$log = Join-Path $notes "server.log"
$err = Join-Path $notes "server.err.log"
$proc = Start-Process -FilePath $exe -ArgumentList "--dir", "`"$notes`"", "--port", $Port -RedirectStandardOutput $log -RedirectStandardError $err -NoNewWindow -PassThru

$failures = @()
function Check($name, $condition, $detail) {
  if ($condition) { "  [PASS] $name" }
  else { "  [FAIL] $name  $detail"; $script:failures += $name }
}

try {
  $token = $null
  $deadline = (Get-Date).AddSeconds(20)
  while (-not $token -and (Get-Date) -lt $deadline) {
    if (Test-Path $log) {
      $raw = Get-Content $log -Raw
      if ($raw) {
        $m = [regex]::Match($raw, 'token=([0-9a-f]{64})')
        if ($m.Success) { $token = $m.Groups[1].Value }
      }
    }
    Start-Sleep -Milliseconds 100
  }
  if (-not $token) { throw "服务未在 20 秒内就绪`n$(Get-Content $err -Raw)" }

  $base = "http://127.0.0.1:$Port"
  $session = New-Object Microsoft.PowerShell.Commands.WebRequestSession

  "1. 鉴权"
  Check "无 Cookie 访问 /api/todos 返回 401" ((Invoke-WebRequest -Uri "$base/api/todos" -SkipHttpErrorCheck).StatusCode -eq 401)
  $page = Invoke-WebRequest -Uri "$base/?token=$token" -WebSession $session
  Check "带 token 引导返回页面" ($page.StatusCode -eq 200 -and $page.Content -match '<div class="app"')
  Check "Cookie 已种" ([bool]$session.Cookies.GetCookies($base)['nc_token'])
  $againStatus = (& curl.exe -s -o NUL -w "%{http_code}" -H "Cookie: nc_token=$token" "$base/?token=$token").Trim()
  Check "已有 Cookie 时 ?token 被拒绝（307 重定向）" ($againStatus -eq '307') "实际 $againStatus"

  "2. 扫描与解析"
  $todos = Invoke-RestMethod -Uri "$base/api/todos" -WebSession $session
  Check "未完成待办 4 条" ($todos.total -eq 4) "实际 $($todos.total)"
  Check "代码块里的待办未被收录" (-not ($todos.items | Where-Object { $_.text -match '代码块里' }))

  "3. 日程"
  $sch = Invoke-RestMethod -Uri "$base/api/schedule?range=week" -WebSession $session
  $day = $sch.days | Where-Object { $_.date -eq $dateStr }
  Check "当日日程 2 条" ($day.items.Count -eq 2) "实际 $($day.items.Count)"
  Check "未识别行进诊断区" ($day.unrecognized.Count -eq 1)

  "4. 标签"
  $tags = Invoke-RestMethod -Uri "$base/api/tags" -WebSession $session
  Check "标签树顶层含 工作" ([bool]($tags.tree | Where-Object { $_.name -eq '工作' }))

  "5. 搜索"
  $sr = Invoke-RestMethod -Uri "$base/api/search?q=$([uri]::EscapeDataString('采购'))" -WebSession $session
  Check "搜索命中" ($sr.total -ge 1)
  $srDone = Invoke-RestMethod -Uri "$base/api/search?q=$([uri]::EscapeDataString('采购'))&status=open" -WebSession $session
  Check "状态过滤排除已完成" (-not ($srDone.items | Where-Object { $_.done })) "命中 $($srDone.total) 条"

  "6. 待办回写（三级定位）"
  $file = Join-Path $notes $relWin
  $text = Get-Content $file -Raw
  Set-Content -Path $file -Value ($text.Replace("## 待办`n", "## 待办`n`n外部插入的一行`n")) -NoNewline -Encoding utf8
  $target = (Invoke-RestMethod -Uri "$base/api/todos" -WebSession $session).items | Where-Object { $_.text -match '发供应链周报' }
  $body = @{ path = $target.path; line = $target.line; raw_line = $target.raw_line; done = $true } | ConvertTo-Json
  $patched = Invoke-RestMethod -Uri "$base/api/todos" -Method Patch -WebSession $session -ContentType "application/json; charset=utf-8" -Body ([Text.Encoding]::UTF8.GetBytes($body))
  $after = Get-Content $file -Raw
  Check "插入行后仍改对行" ($after -match '- \[x\] 发供应链周报.*✅')
  Check "外部插入内容未被破坏" ($after -match '外部插入的一行')
  Check "返回新行号" ($patched.line -gt 0)

  "7. 版本冲突"
  $cur = Invoke-RestMethod -Uri "$base/api/files/$([uri]::EscapeDataString($relPosix))" -WebSession $session
  $save = @{ content = "改一下`n"; version = "0-0" } | ConvertTo-Json
  $conflict = Invoke-WebRequest -Uri "$base/api/files/$([uri]::EscapeDataString($relPosix))" -Method Put -WebSession $session -ContentType "application/json" -Body ([Text.Encoding]::UTF8.GetBytes($save)) -SkipHttpErrorCheck
  Check "旧版本号保存返回 409" ($conflict.StatusCode -eq 409)
  $forced = Invoke-WebRequest -Uri "$base/api/files/$([uri]::EscapeDataString($relPosix))?force=true" -Method Put -WebSession $session -ContentType "application/json" -Body ([Text.Encoding]::UTF8.GetBytes($save)) -SkipHttpErrorCheck
  Check "force 覆盖返回 200" ($forced.StatusCode -eq 200)

  "8. 静态资源与路径防护"
  $img = Invoke-WebRequest -Uri "$base/api/asset?path=$([uri]::EscapeDataString('assets/pic.png'))&from=$([uri]::EscapeDataString('raw/KPI口径.md'))" -WebSession $session -SkipHttpErrorCheck
  Check "图片双轨命中 200" ($img.StatusCode -eq 200)
  $bad = Invoke-WebRequest -Uri "$base/api/asset?path=$([uri]::EscapeDataString('../../../Cargo.toml'))" -WebSession $session -SkipHttpErrorCheck
  Check "越界资源 404" ($bad.StatusCode -eq 404)
  $trav = Invoke-WebRequest -Uri "$base/api/files/$([uri]::EscapeDataString('../Cargo.toml'))" -WebSession $session -SkipHttpErrorCheck
  Check "路径穿越 400" ($trav.StatusCode -eq 400)

  "9. 每日文件模板"
  Remove-Item -LiteralPath (Join-Path $notes $relWin) -Force
  $daily = Invoke-RestMethod -Uri "$base/api/daily" -Method Post -WebSession $session
  Check "按模板新建当天文件" ($daily.created -eq $true -and $daily.content -match '## 日程' -and $daily.content -match '## 待办')
}
finally {
  Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
}

if ($failures.Count) {
  "`n失败 $($failures.Count) 项: $($failures -join ', ')"
  exit 1
}
"`n全部通过"
exit 0
