# 用 Chrome DevTools Protocol 在真实浏览器里做交互验证
# 用法：pwsh -File tools/browser-probe.ps1 -Url "http://127.0.0.1:18086/?token=..." -Hash "#/file/raw/x.md" -Scenario hover|todo|autosave|conflict|resize|all
param(
  [Parameter(Mandatory = $true)][string]$Url,
  [string]$Hash = "",
  [ValidateSet("hover", "todo", "autosave", "conflict", "resize", "search", "tag", "preview", "virtual", "quickadd", "schedadd", "schedjump", "export", "settings", "clicks", "tagtoggle", "scrollkeep", "gitnotice", "all")]
  [string]$Scenario = "all",
  [string]$NotesDir = "",
  [int]$DebugPort = 9333
)

$ErrorActionPreference = "Stop"
$edge = @(
  "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw "找不到 msedge.exe" }

$profile = Join-Path $env:TEMP ("nc-cdp-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
$proc = Start-Process -FilePath $edge -ArgumentList @(
  "--headless=new", "--disable-gpu", "--remote-debugging-port=$DebugPort",
  "--user-data-dir=$profile", "--window-size=1440,1000", "about:blank"
) -PassThru -NoNewWindow

function Send-Cdp($ws, $id, $method, $params) {
  $payload = @{ id = $id; method = $method; params = $params } | ConvertTo-Json -Compress -Depth 8
  $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
  $ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).Wait()
}

function Receive-Cdp($ws, $timeoutMs = 5000) {
  $buffer = New-Object byte[] 262144
  $cts = New-Object Threading.CancellationTokenSource
  $cts.CancelAfter($timeoutMs)
  try {
    $result = $ws.ReceiveAsync([ArraySegment[byte]]::new($buffer), $cts.Token).Result
    if ($result.Count -gt 0) { return [Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count) }
  } catch { }
  return $null
}

function Eval-Js($ws, $expression) {
  $script:cdpSeq++
  Send-Cdp $ws $script:cdpSeq "Runtime.evaluate" @{ expression = $expression; returnByValue = $true; awaitPromise = $true }
  $deadline = (Get-Date).AddSeconds(20)
  while ((Get-Date) -lt $deadline) {
    $msg = Receive-Cdp $ws 1500
    if (-not $msg) { continue }
    $obj = $msg | ConvertFrom-Json
    if ($obj.id -eq $script:cdpSeq) {
      if ($obj.result -and $obj.result.exceptionDetails) {
        $desc = $obj.result.exceptionDetails.exception.description
        return "JS-ERROR: " + ([string]$desc).Split("`n")[0]
      }
      if ($obj.result -and $obj.result.result) { return $obj.result.result.value }
      return $null
    }
  }
  return $null
}

# 每个场景前重新加载页面，避免上一个场景的状态干扰
function Reload-Page($ws, $url) {
  $script:cdpSeq++
  Send-Cdp $ws $script:cdpSeq "Page.navigate" @{ url = $url }
  Start-Sleep -Seconds 2
}
# 真·键盘输入：先用鼠标点进编辑器，再用 CDP Input.insertText
function EnsureEdit($ws) {
  $script:cdpSeq++
  $null = Eval-Js $ws "(() => { const b = Array.from(document.querySelectorAll('.doc-head .seg')).find((x) => x.textContent.trim() === '编辑'); if (b) b.click(); return 'ok'; })()"
  Start-Sleep -Milliseconds 700
}

function Focus-And-Type($ws, $text) {
  EnsureEdit $ws
  $script:cdpSeq++
  $rect = Eval-Js $ws "(() => { const c = document.querySelector('.cm-content'); if (!c) return null; c.focus(); const r = c.getBoundingClientRect(); return JSON.stringify({ x: Math.round(r.left + 120), y: Math.round(r.top + 40) }); })()" $id
  if (-not $rect) { return "no-editor" }
  $p = $rect | ConvertFrom-Json
  $script:cdpSeq++
  Send-Cdp $ws $script:cdpSeq "Input.dispatchMouseEvent" @{ type = "mousePressed"; x = $p.x; y = $p.y; button = "left"; clickCount = 1 }
  $script:cdpSeq++
  Send-Cdp $ws $script:cdpSeq "Input.dispatchMouseEvent" @{ type = "mouseReleased"; x = $p.x; y = $p.y; button = "left"; clickCount = 1 }
  Start-Sleep -Milliseconds 200
  $script:cdpSeq++
  Send-Cdp $ws $script:cdpSeq "Input.insertText" @{ text = $text }
  Start-Sleep -Milliseconds 1600
  $script:cdpSeq++
  return Eval-Js $ws "document.querySelector('.status')?.textContent || ''"
}

try {
  $targets = $null
  $deadline = (Get-Date).AddSeconds(15)
  while ((Get-Date) -lt $deadline) {
    try {
      $targets = Invoke-RestMethod -Uri "http://127.0.0.1:$DebugPort/json/list" -TimeoutSec 2
      if ($targets) { break }
    } catch { Start-Sleep -Milliseconds 300 }
  }
  if (-not $targets) { throw "DevTools 端口未就绪" }
  $page = $targets | Where-Object { $_.type -eq "page" } | Select-Object -First 1
  if (-not $page) { throw "没有可用的页面 target" }

  $ws = New-Object System.Net.WebSockets.ClientWebSocket
  $ws.ConnectAsync([Uri]$page.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait()

  $script:cdpSeq = 0
  Send-Cdp $ws 0 "Page.enable" @{}
  Send-Cdp $ws 0 "Runtime.enable" @{}
  $script:cdpSeq = 10
  Send-Cdp $ws $script:cdpSeq "Page.navigate" @{ url = ($Url + $Hash) }
  Start-Sleep -Seconds 3

  $results = @()
  $want = { param($name) $Scenario -eq "all" -or $Scenario -eq $name }

  if (& $want "hover") {
    Reload-Page $ws ($Url + $Hash)
    EnsureEdit $ws
    # PR #3 起默认进入实时预览编辑态，光标会落到「第一行正文」；闸门 fixture 的正文首行
    # 恰好是双链行，而 live preview 的活动行显示源文本、不生成 .cm-wikilink widget。
    # 用真实点击把光标移出该行（selectionSet 会重算装饰），再测 hover tooltip。
    $script:cdpSeq++
    $rect = Eval-Js $ws "(() => { const c = document.querySelector('.cm-content'); if (!c) return null; const r = c.getBoundingClientRect(); return JSON.stringify({ x: Math.round(r.left + 120), y: Math.round(r.top + 12) }); })()"
    if ($rect -and $rect -notlike 'JS-ERROR*') {
      $p = $rect | ConvertFrom-Json
      $script:cdpSeq++
      Send-Cdp $ws $script:cdpSeq "Input.dispatchMouseEvent" @{ type = "mousePressed"; x = $p.x; y = $p.y; button = "left"; clickCount = 1 }
      $script:cdpSeq++
      Send-Cdp $ws $script:cdpSeq "Input.dispatchMouseEvent" @{ type = "mouseReleased"; x = $p.x; y = $p.y; button = "left"; clickCount = 1 }
      Start-Sleep -Milliseconds 400
    }
    $script:cdpSeq++
    $js = @'
(async () => {
  const span = document.querySelector('.cm-wikilink');
  if (!span) return 'no-wikilink';
  const r = span.getBoundingClientRect();
  const x = r.left + Math.min(20, r.width / 2), y = r.top + r.height / 2;
  for (const type of ['pointerover', 'mouseover', 'pointermove', 'mousemove']) {
    span.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y }));
  }
  await new Promise((r2) => setTimeout(r2, 800));
  const tip = document.querySelector('.cm-tooltip');
  return tip ? (tip.innerText || '').slice(0, 120) : 'no-tooltip';
})()
'@
    $results += "hover => " + (Eval-Js $ws $js )
  }

  if (& $want "todo") {
    Reload-Page $ws ($Url + $Hash) 
    EnsureEdit $ws 
    $script:cdpSeq++
    $js = @'
(async () => {
  const box = document.querySelector('.cm-todo-box');
  if (!box) return 'no-checkbox';
  const before = box.checked;
  box.click();
  await new Promise((r) => setTimeout(r, 900));
  return JSON.stringify({ before, status: document.querySelector('.status')?.textContent || '' });
})()
'@
    $results += "todo => " + (Eval-Js $ws $js )
  }

  if (& $want "autosave") {
    Reload-Page $ws ($Url + $Hash) 
    $typed = Focus-And-Type $ws "`n<!-- autosave-marker -->`n" 
    Start-Sleep -Milliseconds 500
    $script:cdpSeq++
    $verify = Eval-Js $ws @'
(async () => {
  const path = decodeURIComponent(location.hash.replace('#/file/', ''));
  const resp = await fetch('/api/files/' + encodeURIComponent(path));
  const data = await resp.json();
  return JSON.stringify({ status: document.querySelector('.status')?.textContent || '', marker: (data.content || '').includes('autosave-marker') });
})()
'@ 
    $results += "autosave => status=$typed, verify=$verify"
  }

  if (& $want "conflict" -and $NotesDir) {
    Reload-Page $ws ($Url + $Hash) 
    # 页面已加载后，外部改文件使版本号失效
    $rel = [uri]::UnescapeDataString(($Hash -replace '^#/file/', ''))
    $disk = Join-Path $NotesDir $rel.Replace('/', '\')
    if (Test-Path $disk) {
      Add-Content -Path $disk -Value "`n外部改了一行`n" -Encoding utf8
      Start-Sleep -Milliseconds 300
      $typed = Focus-And-Type $ws "`n冲突测试`n" 
      $script:cdpSeq++
      $banner = Eval-Js $ws "document.querySelector('.conflict-bar') ? document.querySelector('.conflict-bar').innerText.slice(0,80) : 'no-banner'" 
      $results += "conflict => status=$typed, banner=$banner"
    } else {
      $results += "conflict => 跳过（找不到文件 $disk）"
    }
  }

  if (& $want "search") {
    Reload-Page $ws ($Url + "#/search/" + [uri]::EscapeDataString("测试")) 
    $script:cdpSeq++
    $js = @'
(async () => {
  const li = document.querySelector('.results li');
  if (!li) return 'no-result';
  li.click();
  await new Promise((r) => setTimeout(r, 1200));
  return JSON.stringify({
    hash: decodeURIComponent(location.hash).slice(0, 60),
    title: document.querySelector('.doc-head .title')?.textContent || '',
    selected: (window.getSelection()?.toString() || '').slice(0, 40),
  });
})()
'@
    $results += "search => " + (Eval-Js $ws $js )
  }

  if (& $want "tag") {
    Reload-Page $ws ($Url + "#/tag/" + [uri]::EscapeDataString("测试")) 
    $script:cdpSeq++
    $js = @'
(async () => {
  const right = document.querySelector('.doc-head .right')?.textContent || '';
  const rows = document.querySelectorAll('table.todos tr[data-path]').length;
  const notes = document.querySelectorAll('.results li').length;
  const carets = document.querySelectorAll('.tag-node .caret').length;
  return JSON.stringify({ right, rows, notes, carets });
})()
'@
    $results += "tag => " + (Eval-Js $ws $js )
  }

  if (& $want "tagtoggle") {
    Reload-Page $ws ($Url + "#/")
    $js = @'
(async () => {
  const rows = Array.from(document.querySelectorAll('.tag-node'));
  const row = rows.find((r) => r.nextElementSibling && r.nextElementSibling.classList.contains('tag-children'));
  if (!row) return 'no-expandable-tag';
  const sub = row.nextElementSibling;
  const before = sub.hidden;
  row.click();
  await new Promise((r) => setTimeout(r, 300));
  const after = sub.hidden;
  row.click();
  await new Promise((r) => setTimeout(r, 300));
  return JSON.stringify({ before, afterFirstClick: after, afterSecondClick: sub.hidden });
})()
'@
    $results += "tagtoggle => " + (Eval-Js $ws $js)
  }

  if (& $want "scrollkeep") {
    Reload-Page $ws ($Url + "#/")
    $js = @'
(async () => {
  const main = document.querySelector('.main');
  main.scrollTop = 600;
  await new Promise((r) => setTimeout(r, 200));
  const before = main.scrollTop;
  const sel = Array.from(document.querySelectorAll('.todo-filter')).find((s) => s.tagName === 'SELECT' && Array.from(s.options).some((o) => o.textContent.includes('按优先级')));
  if (!sel) return 'no-sort-select';
  sel.value = 'priority';
  sel.dispatchEvent(new Event('change', { bubbles: true }));
  await new Promise((r) => setTimeout(r, 900));
  return JSON.stringify({ before, after: main.scrollTop });
})()
'@
    $results += "scrollkeep => " + (Eval-Js $ws $js)
  }

  if (& $want "preview") {
    Reload-Page $ws ($Url + $Hash)
    $js = @'
(() => {
  const findBtn = () =>
    Array.from(document.querySelectorAll('.doc-head .seg')).find((b) =>
      ['预览', '编辑'].includes(b.textContent.trim())
    );
  const inEditor = () => !!document.querySelector('.editor-host:not([hidden]) .cm-editor');
  const btn = findBtn();
  if (!btn) return 'no-toggle-button';
  // PR #3（0e7aee4）后默认是实时预览编辑态，「预览」按钮降级为只读渲染视图开关。
  // 切换是同步 DOM 操作，不用 awaitPromise，避免连续切换时 evaluate 拿不到结果。
  const stats = { defaultEditing: inEditor() };
  btn.click();
  const article = document.querySelector('.doc.preview');
  const firstBox = article ? article.querySelector('input.preview-todo') : null;
  stats.readonlyPreview = !!(article && !article.hidden);
  stats.headings = article ? article.querySelectorAll('h1,h2,h3').length : 0;
  stats.tables = article ? article.querySelectorAll('table').length : 0;
  stats.images = article ? article.querySelectorAll('img.preview-img').length : 0;
  stats.links = article ? article.querySelectorAll('a').length : 0;
  // 预览里的待办必须是真实、可勾选的复选框（#2）
  stats.todoBoxes = article ? article.querySelectorAll('input.preview-todo').length : 0;
  stats.todoIsInput = !!(firstBox && firstBox.tagName === 'INPUT' && firstBox.type === 'checkbox' && !firstBox.disabled);
  // 只读视图 -> 编辑态 -> 再回只读视图，验证往返不丢内容
  const btn2 = findBtn();
  if (btn2) btn2.click();
  stats.editorVisible = inEditor();
  const btn3 = findBtn();
  if (btn3) btn3.click();
  stats.previewAgain = !!document.querySelector('.doc.preview:not([hidden])');
  return JSON.stringify(stats);
})()
'@
    $results += "preview => " + (Eval-Js $ws $js)
  }
  if (& $want "virtual") {
    Reload-Page $ws ($Url + "#/") 
    $script:cdpSeq++
    $js = @'
(async () => {
  const main = document.querySelector('.main');
  const count = () => document.querySelectorAll('table.todos tr[data-path]').length;
  const first = count();
  const total = (document.querySelector('.doc-head .right')?.textContent || '').replace(/[^0-9]/g, '');
  for (let i = 0; i < 1; i += 1) {
    main.scrollTop = main.scrollHeight;
    await new Promise((r) => setTimeout(r, 600));
  }
  return JSON.stringify({ first, afterScroll: count(), total });
})()
'@
    $results += "virtual => " + (Eval-Js $ws $js )
  }

  if (& $want "clicks") {
    Reload-Page $ws ($Url + $Hash) 
    EnsureEdit $ws 
    $script:cdpSeq++
    $js = @'
(async () => {
  const out = {};
  const start = location.hash;
  const link = document.querySelector('.cm-link');
  if (link) {
    link.click();
    await new Promise((r) => setTimeout(r, 900));
    out.afterLink = decodeURIComponent(location.hash);
  }
  // 回到起始文件再测双链
  location.hash = start;
  await new Promise((r) => setTimeout(r, 1000));
  const wiki = document.querySelector('.cm-wikilink');
  if (!wiki) return JSON.stringify(Object.assign(out, { wikiFound: false }));
  wiki.click();
  await new Promise((r) => setTimeout(r, 1000));
  out.afterWiki = decodeURIComponent(location.hash);
  out.wikiFound = true;
  return JSON.stringify(out);
})()
'@
    $results += "clicks => " + (Eval-Js $ws $js )
  }

  if (& $want "quickadd") {
    Reload-Page $ws ($Url + "#/") 
    $script:cdpSeq++
    $filled = Eval-Js $ws "(() => { const i = document.querySelector('.todos-section .quick-add'); if (!i) return 'no-input'; i.scrollIntoView({ block: 'center' }); i.focus(); const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; setter.call(i, '探针加的待办 #探针'); i.dispatchEvent(new Event('input', { bubbles: true })); i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })); return 'dispatched'; })()" 
    Start-Sleep -Milliseconds 2000
    $script:cdpSeq++
    $check = Eval-Js $ws "JSON.stringify({ filled: '$filled', added: !!Array.from(document.querySelectorAll('table.todos .text')).find((e) => e.textContent.includes('探针加的待办')), total: document.querySelector('.doc-head .right')?.textContent || '' })" 
    $results += "quickadd => " + $check
  }

  if (& $want "schedjump") {
    Reload-Page $ws ($Url + "#/") 
    $script:cdpSeq++
    $js = @'
(async () => {
  const li = Array.from(document.querySelectorAll('.slots li')).find((e) => e.style.cursor === 'pointer');
  if (!li) return 'no-clickable-item';
  const label = li.textContent.slice(0, 40);
  li.click();
  await new Promise((r) => setTimeout(r, 1200));
  return JSON.stringify({
    label,
    hash: decodeURIComponent(location.hash).slice(0, 70),
    title: document.querySelector('.doc-head .title')?.textContent || '',
    selected: (window.getSelection()?.toString() || '').slice(0, 30),
  });
})()
'@
    $results += "schedjump => " + (Eval-Js $ws $js )
  }

  if (& $want "export") {
    Reload-Page $ws ($Url + $Hash) 
    $script:cdpSeq++
    $js = @'
(() => {
  // PDF 导出已按用户反馈隐藏：这里断言头部不再出现「导出 PDF / 打印」按钮
  const btns = Array.from(document.querySelectorAll('.doc-head button'))
    .filter((b) => /导出\s*PDF|打印/.test(b.textContent));
  return JSON.stringify({ exportButtons: btns.length });
})()
'@
    $results += "export => " + (Eval-Js $ws $js )
  }

  if (& $want "settings") {
    Reload-Page $ws ($Url + "#/")
    $script:cdpSeq++
    $js = @'
(async () => {
  const btn = document.getElementById('settings');
  if (!btn) return 'no-settings-button';
  btn.click();
  await new Promise((r) => setTimeout(r, 300));
  const modal = document.querySelector('.settings-modal');
  const selects = modal ? modal.querySelectorAll('select').length : 0;
  let themeApplied = false;
  const themeSel = modal ? modal.querySelectorAll('select')[2] : null;
  if (themeSel) {
    themeSel.value = 'dark';
    themeSel.dispatchEvent(new Event('change', { bubbles: true }));
    themeApplied = document.documentElement.getAttribute('data-theme') === 'dark';
    themeSel.value = 'auto';
    themeSel.dispatchEvent(new Event('change', { bubbles: true }));
  }
  const modalOpen = !!document.querySelector('.settings-modal');
  if (modal && modal.closest('.modal-mask')) modal.closest('.modal-mask').remove();
  return JSON.stringify({ modalOpen, selects, themeApplied });
})()
'@
    $results += "settings => " + (Eval-Js $ws $js)
  }

  if (& $want "gitnotice") {
    Reload-Page $ws ($Url + "#/")
    $script:cdpSeq++
    $js = @'
(() => {
  const n = document.getElementById('git-notice');
  if (!n) return JSON.stringify({ gitNotice: false, reason: 'missing' });
  const r = n.getBoundingClientRect();
  const cs = getComputedStyle(n);
  const text = (n.textContent || '').trim();
  return JSON.stringify({
    gitNotice: r.width > 0 && r.height > 0 && cs.visibility === 'visible' && cs.display !== 'none' && text.includes('仅本地版本历史，无远程备份'),
    width: Math.round(r.width),
    height: Math.round(r.height),
    text: text
  });
})()
'@
    $results += "gitnotice => " + (Eval-Js $ws $js)
  }

  if (& $want "schedadd") {
    Reload-Page $ws ($Url + "#/") 
    $script:cdpSeq++
    $filled = Eval-Js $ws "(() => { const row = document.querySelector('.schedule .schedule-add'); if (!row) return 'no-form'; row.classList.add('is-open'); const start = row.querySelector('input[type=time]'); const title = row.querySelector('.quick-add.title'); const btn = row.querySelector('.action'); if (!start || !title || !btn) return 'no-inputs'; const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; start.scrollIntoView({ block: 'center' }); setter.call(start, '11:00'); start.dispatchEvent(new Event('input', { bubbles: true })); setter.call(title, '闸门加的日程'); title.dispatchEvent(new Event('input', { bubbles: true })); btn.click(); return 'dispatched'; })()" 
    Start-Sleep -Milliseconds 2200
    $script:cdpSeq++
    $check = Eval-Js $ws "JSON.stringify({ filled: '$filled', added: !!Array.from(document.querySelectorAll('.slots .what')).find((e) => e.textContent.includes('闸门加的日程')), items: document.querySelectorAll('.slots li').length })" 
    $results += "schedadd => " + $check
  }

  if (& $want "resize") {
    # 回首页测列宽拖拽
    $script:cdpSeq++
    Send-Cdp $ws $script:cdpSeq "Page.navigate" @{ url = ($Url + "#/") }
    Start-Sleep -Seconds 2
    $script:cdpSeq++
    $js = @'
(() => {
  const handle = document.querySelector('.col-resizer');
  const col = document.querySelector('table.todos col:nth-child(3)');
  if (!handle || !col) return 'no-handle';
  const before = Math.round(col.getBoundingClientRect().width);
  const r = handle.getBoundingClientRect();
  const at = (x) => ({ bubbles: true, clientX: x, clientY: r.top + 5 });
  handle.dispatchEvent(new MouseEvent('mousedown', at(r.left + 2)));
  document.dispatchEvent(new MouseEvent('mousemove', at(r.left + 82)));
  document.dispatchEvent(new MouseEvent('mouseup', at(r.left + 82)));
  const after = Math.round(col.getBoundingClientRect().width);
  return JSON.stringify({ before, after, saved: localStorage.getItem('nc.todoCols.v2') });
})()
'@
    $results += "resize => " + (Eval-Js $ws $js )
  }

  "---- 结果 ----"
  $results | ForEach-Object { $_ }
  $ws.Dispose()
  if ($results -match '(?m)=> (no-|JS-ERROR)') {
    Write-Host "存在失败场景（no-* / JS-ERROR）"
    exit 1
  }
}
finally {
  Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 300
  Remove-Item $profile -Recurse -Force -ErrorAction SilentlyContinue
}
