# 用 Chrome DevTools Protocol 在真实浏览器里做交互验证（悬停、点击）
# 用法：pwsh -File tools/browser-probe.ps1 -Url "http://127.0.0.1:18086/?token=..." -Hash "#/file/raw/xxx.md"
param(
  [Parameter(Mandatory = $true)][string]$Url,
  [string]$Hash = "",
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
  $segment = [ArraySegment[byte]]::new($bytes)
  $ws.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).Wait()
}

function Receive-Cdp($ws, $timeoutMs = 5000) {
  $buffer = New-Object byte[] 262144
  $segment = [ArraySegment[byte]]::new($buffer)
  $cts = New-Object Threading.CancellationTokenSource
  $cts.CancelAfter($timeoutMs)
  try {
    $result = $ws.ReceiveAsync($segment, $cts.Token).Result
    if ($result.Count -gt 0) {
      return [Text.Encoding]::UTF8.GetString($buffer, 0, $result.Count)
    }
  } catch { }
  return $null
}

function Eval-Js($ws, $expression, [ref]$id) {
  $id.Value++
  Send-Cdp $ws $id.Value "Runtime.evaluate" @{ expression = $expression; returnByValue = $true; awaitPromise = $true }
  $deadline = (Get-Date).AddSeconds(6)
  while ((Get-Date) -lt $deadline) {
    $msg = Receive-Cdp $ws 1500
    if (-not $msg) { continue }
    $obj = $msg | ConvertFrom-Json
    if ($obj.id -eq $id.Value) {
      if ($obj.result -and $obj.result.result) { return $obj.result.result.value }
      return $null
    }
  }
  return $null
}

try {
  # 等 DevTools 端口就绪
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

  $id = 0
  Send-Cdp $ws 0 "Page.enable" @{}
  Send-Cdp $ws 0 "Runtime.enable" @{}
  $id = 10
  $target = $Url + $Hash
  Send-Cdp $ws $id "Page.navigate" @{ url = $target }
  Start-Sleep -Seconds 3

  $id++
  $title = Eval-Js $ws "document.querySelector('.doc-head .title')?.textContent || ''" ([ref]$id)
  "页面标题: $title"

  # 1) 悬停到第一个双链上，看是否出现悬浮卡片
  $id++
  $hoverScript = @'
(async () => {
  const span = document.querySelector('.cm-wikilink');
  if (!span) return 'no-wikilink';
  const r = span.getBoundingClientRect();
  const x = r.left + Math.min(20, r.width / 2);
  const y = r.top + r.height / 2;
  for (const type of ['pointerover', 'mouseover', 'pointermove', 'mousemove']) {
    span.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y }));
  }
  await new Promise((r2) => setTimeout(r2, 700));
  const tip = document.querySelector('.cm-tooltip');
  if (!tip) return 'no-tooltip';
  return (tip.innerText || tip.textContent || '').slice(0, 200);
})()
'@
  $hover = Eval-Js $ws $hoverScript ([ref]$id)
  "悬浮预览: $hover"

  # 2) 点击第一个待办复选框，看是否发请求成功（返回行号变化）
  $id++
  $clickScript = @'
(async () => {
  const box = document.querySelector('.cm-todo-box');
  if (!box) return 'no-checkbox';
  const before = box.checked;
  box.click();
  await new Promise((r) => setTimeout(r, 900));
  return JSON.stringify({ before, after: document.querySelector('.status')?.textContent || '' });
})()
'@
  $click = Eval-Js $ws $clickScript ([ref]$id)
  "点击待办: $click"

  $ws.Dispose()
}
finally {
  Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 300
  Remove-Item $profile -Recurse -Force -ErrorAction SilentlyContinue
}
