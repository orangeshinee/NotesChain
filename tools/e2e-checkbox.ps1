# 待办勾选端到端回归（Playwright 驱动）
#
# 用法：pwsh -File tools\e2e-checkbox.ps1 -Url "http://127.0.0.1:8090/?token=...#/file/2026/11/2026-11-01.md#/file/2026/11/2026-11-01.md"
# 断言：点一次 → 恰好 1 行完成标记；再点一次 → 0 行（不会双重切换/静默失败）

param(
  [Parameter(Mandatory = $true)][string]$Url,
  [string]$Session = 'e2e'
)

$ErrorActionPreference = 'Stop'

$js = @'
(async () => {
  const editBtn = Array.from(document.querySelectorAll('.doc-head .seg')).find((x) => x.textContent.trim() === '编辑');
  if (editBtn) { editBtn.click(); await new Promise((r) => setTimeout(r, 500)); }
  const path = decodeURIComponent(location.hash.replace('#/file/', '').split('@')[0]);
  const read = () => fetch('/api/files/' + encodeURIComponent(path)).then((r) => r.json());
  const countDone = (text) => (text.split('\n').filter((l) => l.includes('✅') && /^\s*- \[[xX]\]/.test(l)).length);
  const box1 = document.querySelector('.cm-todo-box');
  if (!box1) return JSON.stringify({ error: 'no-checkbox' });
  const before = await read();
  box1.click();
  await new Promise((r) => setTimeout(r, 1800));
  const mid = await read();
  const box2 = document.querySelector('.cm-todo-box');
  if (!box2) return JSON.stringify({ error: 'no-checkbox-2' });
  box2.click();
  await new Promise((r) => setTimeout(r, 1800));
  const after = await read();
  return JSON.stringify({
    beforeDone: countDone(before.content),
    midDone: countDone(mid.content),
    afterDone: countDone(after.content),
    midOk: countDone(mid.content) === 1,
    afterOk: countDone(after.content) === 0
  });
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
if (-not $line) { Write-Host "FAIL 未能解析结果：`n$raw"; exit 1 }
$json = $line.Trim().Trim('"').Replace('\"', '"') | ConvertFrom-Json
if ($json.error) { Write-Host "FAIL $($json.error)"; exit 1 }
if ($json.midOk -and $json.afterOk) {
  Write-Host "PASS 勾选往返：before=$($json.beforeDone) mid=$($json.midDone) after=$($json.afterDone)"
  exit 0
}
Write-Host "FAIL $line"
exit 1
