# 平替预演：用「Silver Bullet 风格」旧目录验证 PRD 10.1 的关键项
# 用法：pwsh -File tools/legacy-check.ps1 [-Port 18096]
param([int]$Port = 18096)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\release\noteschain.exe"
if (-not (Test-Path $exe)) { throw "先运行 cargo build --release" }

$notes = Join-Path $repo ".demo\legacy-notes"
# 本脚本独占的临时目录：先清空再生成（生成器只接受空目录或有标记的目录）
if (Test-Path $notes) { Remove-Item $notes -Recurse -Force }
& node (Join-Path $PSScriptRoot "legacy-corpus.js") $notes | Out-Null
if ($LASTEXITCODE -ne 0) { throw "语料生成失败（exit=$LASTEXITCODE）" }

# 记录所有既有文件的内容哈希，结束后比对「一个字都没改」
$before = @{}
Get-ChildItem $notes -Recurse -File | Where-Object { $_.FullName -notmatch '\\\.git\\' } | ForEach-Object {
  $rel = $_.FullName.Substring($notes.Length + 1)
  $before[$rel] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash
}

$log = Join-Path $notes "server.log"
$proc = Start-Process -FilePath $exe -ArgumentList "--dir", "`"$notes`"", "--port", $Port -RedirectStandardOutput $log -RedirectStandardError (Join-Path $notes "server.err.log") -NoNewWindow -PassThru

$failures = @()
function Check($name, $ok, $detail) {
  if ($ok) { "  [PASS] $name" } else { "  [FAIL] $name  $detail"; $script:failures += $name }
}

try {
  $token = $null
  $deadline = (Get-Date).AddSeconds(20)
  while (-not $token -and (Get-Date) -lt $deadline) {
    if (Test-Path $log) {
      $raw = Get-Content $log -Raw
      if ($raw) { $m = [regex]::Match($raw, 'token=([0-9a-f]{64})'); if ($m.Success) { $token = $m.Groups[1].Value } }
    }
    Start-Sleep -Milliseconds 100
  }
  if (-not $token) { throw "服务未就绪" }

  $base = "http://127.0.0.1:$Port"
  $s = New-Object Microsoft.PowerShell.Commands.WebRequestSession
  Invoke-WebRequest -Uri "$base/?token=$token" -WebSession $s | Out-Null

  "1. 目录识别（不统一的布局也要认出来）"
  $files = Invoke-RestMethod -Uri "$base/api/files" -WebSession $s
  $diary = $files.groups | Where-Object { $_.kind -eq "diary" }
  $note = $files.groups | Where-Object { $_.kind -eq "note" }
  $diaryPaths = @()
  foreach ($y in $diary.years) { foreach ($m in $y.months) { foreach ($e in $m.entries) { $diaryPaths += $e.path } } }
  $notePaths = @($note.entries | ForEach-Object { $_.path })
  Check "识别出 5 个日记文件" ($diaryPaths.Count -eq 5) ($diaryPaths -join ', ')
  Check "平铺文件被识别为日记" ($diaryPaths -contains '2026-10-01.md') ($diaryPaths -join ', ')
  Check "Daily/ 下的文件被识别为日记" ($diaryPaths -contains 'Daily/2026-10-03.md') ($diaryPaths -join ', ')
  Check "识别出 7 个普通笔记" ($notePaths.Count -eq 7) ($notePaths -join ', ')
  Check "raw/ 子目录笔记被识别" ($notePaths -contains 'raw/子目录/供应链.md') ($notePaths -join ', ')
  Check "frontmatter 日记被识别" ($diaryPaths -contains '2026-10-05.md') ($diaryPaths -join ', ')

  "2. 待办提取"
  $todos = Invoke-RestMethod -Uri "$base/api/todos" -WebSession $s
  Check "未完成待办 14 条" ($todos.total -eq 14) "实际 $($todos.total)"
  Check "代码块里的待办未被收录" (-not ($todos.items | Where-Object { $_.text -match 'query' }))
  Check "todo: 老写法未被当成待办" (-not ($todos.items | Where-Object { $_.text -match '老写法' }))

  "3. todo: 迁移清单"
  $mig = Invoke-RestMethod -Uri "$base/api/migration/todo" -WebSession $s
  Check "迁移清单 2 条" ($mig.total -eq 2) "实际 $($mig.total)"

  "4. 标签"
  $tags = Invoke-RestMethod -Uri "$base/api/tags" -WebSession $s
  $top = @($tags.tree | ForEach-Object { $_.name })
  Check "标签含 工作 / 生活 / 测试" (($top -contains '工作') -and ($top -contains '生活') -and ($top -contains '测试')) ($top -join ',')

  "5. raw/ 同名裸名冲突"
  $cfg = Invoke-RestMethod -Uri "$base/api/config" -WebSession $s
  $dup = @($cfg.duplicateNames | Where-Object { $_.name -eq '供应链' })
  Check "检测到 供应链 同名冲突（2 份）" ($dup.Count -eq 1 -and $dup[0].paths.Count -eq 2) ($cfg.duplicateNames | ConvertTo-Json -Compress)

  "6. 图片双轨解析（旧文件不改一个字）"
  $rel = Invoke-WebRequest -Uri "$base/api/asset?path=$([uri]::EscapeDataString('../assets/a.png'))&from=$([uri]::EscapeDataString('Projects/供应链.md'))" -WebSession $s -SkipHttpErrorCheck
  Check "相对文件自身的写法命中" ($rel.StatusCode -eq 200) "status=$($rel.StatusCode)"
  $root = Invoke-WebRequest -Uri "$base/api/asset?path=$([uri]::EscapeDataString('assets/b.png'))&from=$([uri]::EscapeDataString('Projects/供应链.md'))" -WebSession $s -SkipHttpErrorCheck
  Check "根基准写法兜底命中" ($root.StatusCode -eq 200) "status=$($root.StatusCode)"
  $bad = Invoke-WebRequest -Uri "$base/api/asset?path=$([uri]::EscapeDataString('../../../Cargo.toml'))" -WebSession $s -SkipHttpErrorCheck
  Check "根目录外一律拒绝" ($bad.StatusCode -eq 404) "status=$($bad.StatusCode)"

  "7. 编码兼容"
  $bom = Invoke-RestMethod -Uri "$base/api/files/$([uri]::EscapeDataString('raw/BOM笔记.md'))" -WebSession $s
  Check "BOM 文件内容无不可见前缀" (-not $bom.content.StartsWith([char]0xFEFF))
  $crlf = Invoke-RestMethod -Uri "$base/api/files/$([uri]::EscapeDataString('raw/CRLF笔记.md'))" -WebSession $s
  Check "CRLF 文件被识别（eol=crlf）" ($crlf.eol -eq 'crlf') "eol=$($crlf.eol)"
}
finally {
  Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 500
}

"8. 既有文件是否被改写"
$after = @{}
Get-ChildItem $notes -Recurse -File | Where-Object { $_.FullName -notmatch '\\\.git\\' } | ForEach-Object {
  $rel = $_.FullName.Substring($notes.Length + 1)
  $after[$rel] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash
}
$changed = @()
foreach ($k in $before.Keys) {
  if ($after.ContainsKey($k) -and $after[$k] -ne $before[$k]) { $changed += $k }
  if (-not $after.ContainsKey($k)) { $changed += "$k (被删除)" }
}
$added = @($after.Keys | Where-Object { -not $before.ContainsKey($_) })
Check "已有文件一个都没被改写" ($changed.Count -eq 0) ($changed -join ', ')
Check "只新增了 .gitignore 与 server.log/err" ((($added | Where-Object { $_ -notin @('.gitignore', 'server.log', 'server.err.log') }).Count -eq 0)) ($added -join ', ')

""
if ($failures.Count) {
  "失败 $($failures.Count) 项: $($failures -join ', ')"
  exit 1
}
"平替预演全部通过"
exit 0
