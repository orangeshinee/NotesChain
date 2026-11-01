# 边界场景检查：真实目录往往比演示语料脏得多
# 用法：pwsh -File tools/edge-check.ps1 [-Port 18098]
param([int]$Port = 18098)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\release\noteschain.exe"
if (-not (Test-Path $exe)) { throw "先运行 cargo build --release" }

$root = Join-Path $repo ".demo\edge-notes"
if (Test-Path $root) { Remove-Item $root -Recurse -Force }
New-Item -ItemType Directory -Force -Path $root | Out-Null

& (Join-Path $PSScriptRoot "edge-corpus.ps1") -Root $root

$failures = @()
function Check($name, $ok, $detail) {
  if ($ok) { "  [PASS] $name" } else { "  [FAIL] $name  $detail"; $script:failures += $name }
}

$log = Join-Path $root "server.log"
$proc = Start-Process -FilePath $exe -ArgumentList "--dir", "`"$root`"", "--port", $Port, "--no-git" -RedirectStandardOutput $log -RedirectStandardError (Join-Path $root "server.err.log") -NoNewWindow -PassThru

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

  $files = Invoke-RestMethod -Uri "$base/api/files" -WebSession $s
  $notePaths = @(($files.groups | Where-Object { $_.kind -eq "note" }).entries | ForEach-Object { $_.path })
  $diaryPaths = @()
  foreach ($y in (($files.groups | Where-Object { $_.kind -eq "diary" }).years)) {
    foreach ($m in $y.months) { foreach ($e in $m.entries) { $diaryPaths += $e.path } }
  }

  "1. 怪文件名"
  Check "含空格与中文的文件被收录" ($notePaths -contains '奇怪 名字 的笔记.md') ($notePaths -join ' | ')
  Check "含 # 的文件被收录" ($notePaths -contains '含#井号#的笔记.md') ($notePaths -join ' | ')
  Check "含方括号的文件被收录" ($notePaths -contains '带[方括号]的笔记.md') ($notePaths -join ' | ')
  Check "含 emoji 的文件被收录" ($notePaths -contains '带😀emoji.md') ($notePaths -join ' | ')

  "2. 读取接口对怪名字可用"
  foreach ($p in @('奇怪 名字 的笔记.md', '含#井号#的笔记.md', '带[方括号]的笔记.md', '带😀emoji.md')) {
    $r = Invoke-WebRequest -Uri "$base/api/files/$([uri]::EscapeDataString($p))" -WebSession $s -SkipHttpErrorCheck
    Check "GET $p 返回 200" ($r.StatusCode -eq 200) "status=$($r.StatusCode)"
  }

  "3. 大文件与深层目录"
  Check "1 MB 单文件被收录" ($notePaths -contains '大文件.md') ($notePaths -join ' | ')
  Check "10 层深层文件被收录" ($notePaths -contains 'a/b/c/d/e/f/g/h/i/j/深层.md') ($notePaths -join ' | ')
  $deepTodo = @((Invoke-RestMethod -Uri "$base/api/todos" -WebSession $s).items | Where-Object { $_.path -like '*深层.md' })
  Check "深层目录里的待办被统计" ($deepTodo.Count -ge 1) "count=$($deepTodo.Count)"

  "4. 非 UTF-8 文件"
  $warn = @($files.warnings)
  Check "GBK 文件产生警告而不是崩溃" ($warn.Count -ge 1) ($warn -join ' | ')
  Check "GBK 文件未被收录为笔记" (-not ($notePaths -contains 'GBK笔记.md')) ($notePaths -join ' | ')

  "5. .gitignore 生效"
  Check "drafts/ 下的文件被忽略" (-not ($notePaths -contains 'drafts/草稿.md')) ($notePaths -join ' | ')
  Check "备份文件被 .gitignore 忽略" (-not ($notePaths -contains '草稿.bak.md')) ($notePaths -join ' | ')
  Check "被忽略目录里的待办没进统计" (-not ((Invoke-RestMethod -Uri "$base/api/todos" -WebSession $s).items | Where-Object { $_.path -like 'drafts/*' }))

  "6. 空目录与只有附件的目录"
  $empty = Join-Path $repo ".demo\edge-empty"
  if (Test-Path $empty) { Remove-Item $empty -Recurse -Force }
  New-Item -ItemType Directory -Force -Path (Join-Path $empty "assets") | Out-Null
  Set-Content (Join-Path $empty "assets\a.png") "not a real png" -Encoding ascii
  $p2 = Start-Process -FilePath $exe -ArgumentList "--dir", "`"$empty`"", "--port", ($Port + 1), "--no-git" -RedirectStandardOutput (Join-Path $empty "log.txt") -RedirectStandardError (Join-Path $empty "log.err") -NoNewWindow -PassThru
  Start-Sleep -Seconds 2
  $t2 = $null
  $m2 = [regex]::Match((Get-Content (Join-Path $empty "log.txt") -Raw), 'token=([0-9a-f]{64})')
  if ($m2.Success) { $t2 = $m2.Groups[1].Value }
  Check "空目录也能正常启动" ([bool]$t2)
  if ($t2) {
    $s2 = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    Invoke-WebRequest -Uri "http://127.0.0.1:$($Port + 1)/?token=$t2" -WebSession $s2 | Out-Null
    $f2 = Invoke-RestMethod -Uri "http://127.0.0.1:$($Port + 1)/api/files" -WebSession $s2
    $c2 = Invoke-RestMethod -Uri "http://127.0.0.1:$($Port + 1)/api/todos" -WebSession $s2
    Check "空目录文件列表为 0" ((@($f2.groups | ForEach-Object { if ($_.kind -eq 'note') { $_.entries.Count } else { 0 } }) | Measure-Object -Sum).Sum -eq 0)
    Check "空目录待办为 0" ($c2.total -eq 0) "实际 $($c2.total)"
  }
  Get-Process -Id $p2.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  Remove-Item $empty -Recurse -Force -ErrorAction SilentlyContinue
}
finally {
  Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
}

""
if ($failures.Count) { "失败 $($failures.Count) 项: $($failures -join ', ')"; exit 1 }
"边界场景全部通过"
exit 0
