# 保存中继续输入的端到端回归（Playwright 驱动）
#
# 用法：
#   pwsh -File tools\e2e-savetype.ps1 -Url "http://127.0.0.1:8090/?token=...#/file/2026/11/2026-11-01.md"
#
# 覆盖 bug：保存请求在途时继续输入，旧实现会把新输入误标为已保存并丢失/死锁。
# 断言：最终状态下 status 含「已保存」，且磁盘内容 === 编辑器内容。

param(
  [Parameter(Mandatory = $true)][string]$Url,
  [string]$Session = 'e2e'
)

$ErrorActionPreference = 'Stop'

# playwright-cli 输出 UTF-8，Windows 控制台默认 GB2312；不改这里，管道会把中文 status
# 解成乱码，使下面「已保存」的 -like 恒为 false（业务断言值其实全对）。只对齐接收解码。
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$js = @'
(async () => {
  const btn = Array.from(document.querySelectorAll('.doc-head .seg')).find((x) => x.textContent.trim() === '编辑');
  if (btn) { btn.click(); await new Promise((r) => setTimeout(r, 400)); }
  const view = window.__ncView;
  if (!view) return JSON.stringify({ error: 'no-editor-view' });
  const path = decodeURIComponent(location.hash.replace('#/file/', '').split('@')[0]);
  const before = view.state.doc.length;
  // 连续输入跨越 800ms 自动保存点，且在保存请求进行中继续输入
  for (let i = 0; i < 8; i += 1) {
    view.dispatch({ changes: { from: view.state.doc.length, insert: 'Z' } });
    await new Promise((r) => setTimeout(r, 250));
  }
  await new Promise((r) => setTimeout(r, 2500));
  const res = await fetch('/api/files/' + encodeURIComponent(path)).then((r) => r.json());
  const text = view.state.doc.toString();
  const status = ((document.querySelector('.status') || {}).textContent || '').trim();
  const result = {
    status,
    match: res.content === text,
    grew: text.length - before
  };
  // 清理：删掉本次追加的 Z，恢复原文件
  view.dispatch({ changes: { from: view.state.doc.length - 8, to: view.state.doc.length, insert: '' } });
  await new Promise((r) => setTimeout(r, 1600));
  return JSON.stringify(result);
})()
'@

$nav = $Url
if ($nav -notmatch 'e2e=1') {
  $i = $nav.IndexOf('#')
  if ($i -ge 0) { $nav = $nav.Substring(0, $i) + '&e2e=1' + $nav.Substring($i) } else { $nav = $nav + '&e2e=1' }
}

& playwright-cli -s=$Session open $nav | Out-Null
Start-Sleep -Seconds 2
# 打开测试开关（token 引导会清掉 query，所以用 localStorage），再重载
& playwright-cli -s=$Session eval "localStorage.setItem('nc.e2e','1')" | Out-Null
& playwright-cli -s=$Session reload | Out-Null
Start-Sleep -Seconds 2
$raw = & playwright-cli -s=$Session eval $js
& playwright-cli -s=$Session close | Out-Null

$line = ($raw | Select-String -Pattern '^"\{').Line
if (-not $line) {
  Write-Host "FAIL 未能解析结果：`n$raw"
  exit 1
}
$json = $line.Trim().Trim('"').Replace('\"', '"') | ConvertFrom-Json

# 状态文案可能带非颜色状态前缀（○ / … / ✓ / !），按「含已保存」判断而非起始匹配
$ok = $json.status -like '*已保存*' -and $json.match -eq $true -and $json.grew -eq 8
if ($ok) {
  Write-Host "PASS 保存中继续输入：status=$($json.status) 磁盘一致=$($json.match) 新增=$($json.grew)"
  exit 0
}
Write-Host "FAIL $($line)"
exit 1
