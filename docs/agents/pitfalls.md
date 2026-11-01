# 实现坑（都是真踩过的）

1. **改 `web/src/editor.js` 后必须 `npm run build`**，再 `cargo build`。只 cargo build 的话内嵌的是旧 dist，改了等于没改。
2. **块级装饰只能来自 StateField**。`Decoration.replace({block:true})` 放进 ViewPlugin 会抛错，并让整页装饰全部失效（表格渲染就是这么实现的）。
3. **装饰区间必须有序且不重叠**，否则 `RangeSetBuilder` 抛错 → 整页装饰失效。构建时要排序并跳过重叠区间。
4. **不要给隐藏的 CodeMirror `setValue`**。`display:none` 容器里 CM 会陷入测量死循环，整页卡死（预览因此保持只读）。
5. **Windows 路径**：`canonicalize()` 返回 `\\?\F:\...`，git 不认。统一用 `state::simplify` 去前缀。
6. **路径一律经 `AppState::resolve()`**（规范化 + 根目录包含校验）；不要自己 `root.join()`，会绕过穿越防护。
7. **写完文件别忘了** `st.git.notify()` 和 `st.revision.fetch_add(1)`，否则不提交、界面不刷新。
8. **PowerShell 的 `-Path` 把 `[ ]` 当通配符**（`Set-Content`/`Test-Path` 都中招，报「找不到参数 Encoding」这种怪错）。处理怪文件名用 `-LiteralPath` 或 .NET API。
9. **验证脚本自己也会腐坏**。改前端交互后必须跑 `tools/browser-probe.ps1`：历史上「搜索跳转」「双链 missing 样式」都是接口层全绿但界面静默失效，靠探针才发现。
10. **不要双击表格外的空白期望编辑**：表格是整块装饰，必须双击表格本身（widget 上有 dblclick 把光标放进源码）。
