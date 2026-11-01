# 对真实笔记目录做「只读体检」：不建仓库、不改文件，只报告识别结果
# 用法：pwsh -File tools/verify-real-dir.ps1 -Path "D:\notes" [-Port 18097] [-Out report.md]
param(
  [Parameter(Mandatory = $true)][string]$Path,
  [int]$Port = 18097,
  [string]$Out = ""
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\release\noteschain.exe"
if (-not (Test-Path $exe)) { throw "先运行 cargo build --release" }
if (-not (Test-Path $Path)) { throw "目录不存在: $Path" }
$notes = (Resolve-Path $Path).Path
if (-not (Test-Path (Join-Path $notes "*.md") -PathType Any) -and -not (Get-ChildItem $notes -Recurse -Filter *.md -File -ErrorAction SilentlyContinue)) {
  throw "目录里没有 .md 文件: $notes"
}

"扫描目录：$notes"
"（只读模式：以 --no-git 启动，不创建 .git / .gitignore，不改任何文件）"

# 记录既有文件哈希
$before = @{}
Get-ChildItem $notes -Recurse -File -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -notmatch '\\\.git\\' } |
  ForEach-Object {
    $rel = $_.FullName.Substring($notes.Length + 1)
    $before[$rel] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash
  }
"先记录 $($before.Count) 个文件的哈希"

$log = Join-Path $env:TEMP ("nc-verify-" + [guid]::NewGuid().ToString("N").Substring(0, 6) + ".log")
$proc = Start-Process -FilePath $exe -ArgumentList "--dir", "`"$notes`"", "--port", $Port, "--no-git" -RedirectStandardOutput $log -RedirectStandardError "$log.err" -NoNewWindow -PassThru

$lines = New-Object System.Collections.Generic.List[string]
function Say($text) { $text; $script:lines.Add($text) }

try {
  $token = $null
  $deadline = (Get-Date).AddSeconds(30)
  while (-not $token -and (Get-Date) -lt $deadline) {
    if (Test-Path $log) {
      $raw = Get-Content $log -Raw
      if ($raw) { $m = [regex]::Match($raw, 'token=([0-9a-f]{64})'); if ($m.Success) { $token = $m.Groups[1].Value } }
    }
    Start-Sleep -Milliseconds 150
  }
  if (-not $token) { throw "服务未在 30 秒内就绪（目录太大？看 $log）" }

  $base = "http://127.0.0.1:$Port"
  $s = New-Object Microsoft.PowerShell.Commands.WebRequestSession
  Invoke-WebRequest -Uri "$base/?token=$token" -WebSession $s | Out-Null

  $files = Invoke-RestMethod -Uri "$base/api/files" -WebSession $s
  $diary = $files.groups | Where-Object { $_.kind -eq "diary" }
  $note = $files.groups | Where-Object { $_.kind -eq "note" }
  $diaryPaths = @()
  foreach ($y in $diary.years) { foreach ($m in $y.months) { foreach ($e in $m.entries) { $diaryPaths += $e.path } } }
  $notePaths = @($note.entries | ForEach-Object { $_.path })

  $todos = Invoke-RestMethod -Uri "$base/api/todos?limit=1" -WebSession $s
  $done = Invoke-RestMethod -Uri "$base/api/todos?status=done&limit=1" -WebSession $s
  $tags = Invoke-RestMethod -Uri "$base/api/tags" -WebSession $s
  $mig = Invoke-RestMethod -Uri "$base/api/migration/todo" -WebSession $s
  $cfg = Invoke-RestMethod -Uri "$base/api/config" -WebSession $s
  $sch = Invoke-RestMethod -Uri "$base/api/schedule?range=today" -WebSession $s

  Say ""
  Say "## 识别结果"
  Say ""
  Say "| 项 | 数值 |"
  Say "| --- | --- |"
  Say "| 日记文件 | $($diaryPaths.Count) |"
  Say "| 普通笔记 | $($notePaths.Count) |"
  Say "| 未完成待办 | $($todos.total) |"
  Say "| 已完成待办 | $($done.total) |"
  Say "| 标签顶层数 | $($tags.tree.Count) |"
  Say "| todo: 历史写法 | $($mig.total) |"
  Say "| 同名裸名冲突 | $($cfg.duplicateNames.Count) |"
  Say "| 扫描告警（非 UTF-8 等） | $($files.warnings.Count) |"
  Say "| 今日日程条目 | $(@($sch.days[0].items).Count) |"

  if ($cfg.duplicateNames.Count) {
    Say ""
    Say "### raw/ 同名冲突（需改名消除）"
    foreach ($d in $cfg.duplicateNames) { Say "- ``$($d.name)``: $($d.paths -join ' | ')" }
  }
  if ($files.warnings.Count) {
    Say ""
    Say "### 扫描告警"
    foreach ($w in $files.warnings) { Say "- $w" }
  }
  if ($diaryPaths.Count -eq 0) {
    Say ""
    Say "> 注意：没有识别到日记文件。若你的日记靠 ``# YYYY-MM-DD`` 标题，请检查标题格式。"
  }
}
finally {
  Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 600
  Remove-Item $log, "$log.err" -Force -ErrorAction SilentlyContinue
}

# 比对文件哈希
$after = @{}
Get-ChildItem $notes -Recurse -File -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -notmatch '\\\.git\\' } |
  ForEach-Object {
    $rel = $_.FullName.Substring($notes.Length + 1)
    $after[$rel] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash
  }
$changed = @()
foreach ($k in $before.Keys) {
  if (-not $after.ContainsKey($k)) { $changed += "$k (被删除)" }
  elseif ($after[$k] -ne $before[$k]) { $changed += $k }
}
$added = @($after.Keys | Where-Object { -not $before.ContainsKey($_) })

Say ""
Say "## 文件改动检查"
Say ""
if ($changed.Count -eq 0) { Say "- ✅ 已有文件一个都没被改写（逐个 SHA256 比对）" }
else { Say "- ❌ 以下文件被改动：$($changed -join ', ')" }
if ($added.Count -eq 0) { Say "- ✅ 没有新增文件" } else { Say "- 新增：$($added -join ', ')" }

if ($Out) {
  $lines -join "`n" | Set-Content -Path $Out -Encoding utf8
  "报告已写入 $Out"
}

if ($changed.Count -gt 0) { exit 1 }
exit 0
