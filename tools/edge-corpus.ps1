# 生成边界场景语料（供 edge-check.ps1 使用）
param([Parameter(Mandatory = $true)][string]$Root)

# 注意：文件名里可能有 [ ]，PowerShell 的 -Path 会当通配符，必须用 .NET API 或 -LiteralPath
function Write-Utf8($rel, $content) {
  $file = Join-Path $Root $rel
  $dir = Split-Path -Parent $file
  [IO.Directory]::CreateDirectory($dir) | Out-Null
  [IO.File]::WriteAllText($file, $content, (New-Object Text.UTF8Encoding($false)))
}

Write-Utf8 '2026-10-01.md' "# 2026-10-01`n`n- [ ] 正常待办`n"
Write-Utf8 '奇怪 名字 的笔记.md' "# 空格与中文`n`n- [ ] 怪名字文件里的待办 #测试`n"
Write-Utf8 '含#井号#的笔记.md' "# 井号`n`n- [ ] 井号文件里的待办`n"
Write-Utf8 '带[方括号]的笔记.md' "# 方括号`n`n- [ ] 方括号文件里的待办`n"
Write-Utf8 '带😀emoji.md' "# emoji`n`n- [ ] emoji 文件里的待办`n"

# 1 MB 单文件
$big = "# 大文件`n`n- [ ] 大文件里的待办`n`n" + ("填充内容 " * 40000)
Write-Utf8 '大文件.md' $big

# 10 层深层目录
Write-Utf8 'a/b/c/d/e/f/g/h/i/j/深层.md' "# 深层`n`n- [ ] 深层文件里的待办`n"

# GBK 编码文件（非 UTF-8，应产生告警且不被收录）
$gbk = [Text.Encoding]::GetEncoding(936).GetBytes("# GBK 笔记`n`n- [ ] GBK 里的待办`n")
[IO.File]::WriteAllBytes((Join-Path $Root 'GBK笔记.md'), $gbk)

# .gitignore 与应被忽略的文件
Write-Utf8 '.gitignore' "drafts/
*.bak.md
"
Write-Utf8 'drafts/草稿.md' "# 草稿`n`n- [ ] 草稿里的待办（应被忽略）`n"
Write-Utf8 '草稿.bak.md' "# 备份`n`n- [ ] 备份里的待办（应被忽略）`n"
