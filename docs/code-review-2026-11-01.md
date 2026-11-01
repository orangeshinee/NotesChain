# NotesChain 代码审查报告

- 审查日期：2026-11-01
- 审查基线：`1856b3e0337dce39741203578a752d1cea69292e`（`master`）
- 审查方式：只读代码审查 + 可丢弃的 `.demo` 目录运行验证
- 审查范围：`src/` 全部 Rust 模块、`web/app.js`、`web/src/editor.js`、`tools/`、构建与测试配置、相关产品文档
- 结论：**暂不建议直接用于真实笔记目录。** 架构方向清楚，但现有验证体系无法证明数据安全，且有多条可以把未保存编辑丢失、误报冲突、让已删除内容“复活”的路径。

> 本报告只记录问题，没有修改业务代码、测试脚本或用户笔记。

## 1. 总体判断

项目的边界划分和产品约束写得比较清楚：单二进制、本地优先、按文件写锁、原子写、版本号冲突检测、按天分文件、`.trash` 回收站，这些方向都合理。

主要风险不在“代码风格”，而在四个跨层不变量没有真正做到：

1. **一个文件的版本号必须基于同一份字节表示计算**：当前 GET、PUT、PATCH、索引四处并不一致，BOM 文件会第一次保存就 409，CRLF 文件第二次保存会 409。
2. **编辑器的“已保存”必须代表磁盘上已经包含用户当前全部输入**：当前保存请求只保证“请求发出时的快照”，后续输入会被误标为已保存并在轮询/导航时丢失。
3. **扫描与增量监听必须使用同一套“哪些文件算笔记”的规则**：当前监听绕过了 `.gitignore`，删除事件被忽略，`.trash` 在旧布局下还会被重新收录。
4. **发布闸门的“全绿”必须代表验证真的执行了**：当前浏览器层没有产生任何断言也能返回“全部通过”，构建也可能拿旧 exe 判定成功。

## 2. 验证结果与证据边界

| 验证 | 结果 | 说明 |
| --- | --- | --- |
| `cargo build --release` | 通过 | 2026-11-01 本机构建成功 |
| `cargo test` | 通过 | 26 passed，0 failed |
| `cargo clippy --all-targets` | 通过 | 0 warnings / 0 errors |
| `node --check web/app.js` | 通过 | 语法检查通过 |
| `npm run build` + `git diff --exit-code -- web/dist/editor.js` | 通过 | 重新打包后无差异，当前提交的前端产物与源码一致 |
| `pwsh -File tools/release-gate.ps1` | 退出码 0，显示“全部通过（6 项）” | **但浏览器交互层没有产生任何断言**，见 H9 |

针对关键数据路径，在 `.demo/review-*` 临时目录做了最小复现：

| 复现 | 结果 |
| --- | --- |
| 带 UTF-8 BOM 的文件：GET 后内容不变直接 PUT | `409 conflict`，复现“正常保存被误判冲突” |
| 纯 CRLF 文件：第一次保存返回 200，再用返回的 version 保存同一内容 | 第二次 `409 conflict`，复现 GET/PUT 版本哈希口径不一致 |
| 旧布局目录删除一篇笔记后重建索引 | 文件列表出现 `.trash/20261101-210014-a.md`，待办仍为 1 条，复现“删除后复活” |
| 标准 `notes/` 布局的 Git 历史恢复 | `POST /api/git/restore` 返回 404，复现恢复路径拼接错误 |

证据边界：

- 浏览器交互没有在本轮独立逐项验证；发布闸门中的浏览器层实际没有跑出断言，因此**不能据此认定前端交互通过**。
- 未运行 `tools/gen-corpus.js` / `tools/legacy-corpus.js`，因为其对传入目录执行无保护递归删除。
- 性能、云桌面、网络盘、真实笔记目录未在本轮重测。
- `style.css` 只检查了与安全、隐藏元素相关的部分，没有做视觉回归。
- `web/node_modules` 属于第三方依赖，不在本次逐文件审查范围。

## 3. 问题清单

### 高优先级：可能导致数据丢失、删除复活、核心功能不可用

| 编号 | 位置 | 问题 |
| --- | --- | --- |
| H1 | `web/app.js:967-1029`、`web/app.js:1034-1040`、`web/app.js:1848-1884` | 保存请求只保存快照；在途期间的新输入会被 `dirty=false` 标记为已保存，后续轮询可能把编辑器回退到磁盘旧内容 |
| H2 | `web/app.js:970-972`、`web/app.js:987-1026`、`web/app.js:1630-1651`、`web/app.js:1265-1275` | 导航没有等待“真实保存完成”；保存中、冲突或失败时仍会销毁编辑器，未保存内容随 DOM 消失 |
| H3 | `web/src/editor.js:254-263`、`web/app.js:1068-1078` | 正文未保存时勾选待办，PATCH 只写目标行，却把整篇 `dirty` 清掉，其他编辑不再受保护 |
| H4 | `src/api.rs:268-271`、`src/api.rs:312-314`、`src/api.rs:342`、`src/index.rs:170-184` | BOM/CRLF 文件的 version 口径不一致：GET 去 BOM 后哈希，PUT 对原始 BOM/CRLF 哈希；已实测 BOM 首次保存 409、CRLF 第二次保存 409 |
| H5 | `src/api.rs:363-371`、`src/index.rs:44-60`、`src/index.rs:286-290`、`src/watch.rs:69` | 旧布局（根目录直接放 `.md`）下 `.trash` 未排除，删除后重建索引或监听会把 `.trash/*.md` 重新收进笔记、待办、标签和搜索 |
| H6 | `src/watch.rs:62`、`src/watch.rs:69-79`、`src/index.rs:96-100` | 监听对不存在的路径直接跳过，外部删除/重命名永远不删除索引；监听也不应用扫描时的 `.gitignore` 规则，明确忽略的文件会被增量索引重新加入 |
| H7 | `web/app.js:1799-1826`、`src/api.rs:1120-1153` | 标准 `notes/` 布局中，diff 里的路径是仓库相对的 `notes/raw/a.md`，恢复接口却把它当 notes 根相对路径，拼成 `notes/notes/raw/a.md`；已实测 404 |
| H8 | `tools/gen-corpus.js:6-16`、`tools/legacy-corpus.js:8-15` | 生成器直接 `fs.rmSync(process.argv[2], { recursive: true, force: true })`，无路径白名单、目录标记或确认；误传真实笔记目录会先不可逆删光再写测试语料 |
| H9 | `tools/release-gate.ps1:16-30`、`tools/release-gate.ps1:86-116`、`tools/browser-probe.ps1:427-435` | 发布闸门可在构建失败时因旧 exe 存在而 PASS；浏览器探针没有输出、没有断言也不产生失败，最终仍“全部通过”。本轮实际就是 6 项全绿但浏览器断言为 0 |

### 中优先级：特定场景下错写、越界或验证失真

| 编号 | 位置 | 问题 |
| --- | --- | --- |
| M1 | `src/api.rs:519-543`、`src/index.rs:170-184` | 待办回写按“发现任意 CRLF 就全文转 CRLF”重建整篇；混合 LF/CRLF 文件勾选一行会把其他 LF 行也改成 CRLF，产生整篇 diff（BOM 在 PATCH 路径会保留） |
| M2 | `src/api.rs:790-792` 与 `src/api.rs:307-310` | `append_todo` 直接用请求里的 `rel` 当锁键，其他写接口用 `st.relative(&abs)`；`a.md`、`./a.md`、`raw/../a.md` 可绕过同一文件互斥 |
| M3 | `src/api.rs:649-681` | `create_note` 在拿锁前检查 `abs.exists()`，同路径并发创建都认为“不存在”，后写者覆盖先写者；应锁内二次检查 |
| M4 | `src/state.rs:71-76`、`src/api.rs:918-922` | 重建索引直接扫描后替换整个索引，不与按文件写操作协调；保存与重建交错时可能丢失刚写入的索引项 |
| M5 | `src/state.rs:104-114`、`src/api.rs:1593-1617` | 路径包含校验只规范化“已存在”的父目录，缺失父目录直接按词法路径放行；`assets` 候选也没有 canonicalize。软链接/junction 可让读取或新建写到根目录外 |
| M6 | `src/api.rs:301-337`、`src/api.rs:510-543` | 整文件保存和待办回写没有校验 `.md` 后缀，绕开了 PRD 9.3“写接口只接受笔记根内 `.md`”的写入面约束 |
| M7 | `src/git.rs:212-261` | `git add -A -- <笔记目录>` 后执行无路径限制的 `git commit`；沿用父仓库时会把用户已经暂存的无关改动一起提交，违反 PRD 6.3“提交范围只包含笔记目录内变更” |
| M8 | `web/app.js:873-885`、`web/app.js:961-1032`、`web/app.js:1679`、`web/app.js:1848-1884` | 异步路由和轮询没有请求代号/目标路径二次校验；慢请求返回可覆盖新页面，旧编辑器实例会覆盖全局保存回调 |
| M9 | `web/app.js:1038-1040` | 没有 `beforeunload`/`pagehide` 保护；输入后 800ms 内关闭、刷新，或在途保存期间卸载页面，缓冲直接丢失 |
| M10 | `src/index.rs:277-314`、`src/index.rs:40-66` | `.gitignore` 的不支持规则（`!`、`**`）只写入 `Ignored.warnings`，但 `Index::scan` 没有把它合并到 `index.warnings`；用户看不到“哪些忽略规则其实没生效” |
| M11 | `tools/smoke.ps1:85-104`、`tools/browser-probe.ps1:157-170`、`tools/browser-probe.ps1:312-338` | 断言允许明显回归：待办数量允许 4 或 5、状态过滤只校验 `<=`、autosave 的 `"marker":false` 仍可 PASS、clicks 找不到链接可返回空对象；测试通过不等于行为正确 |

### 低优先级：边界行为与一致性问题

| 编号 | 位置 | 问题 |
| --- | --- | --- |
| L1 | `src/api.rs:182-195` | 已带有效 Cookie 时再次访问 `?token=` 仍会重新种 Cookie 并重定向；PRD 9.1 要求此后带 token 的 URL 一律拒绝并重定向。没有发现鉴权绕过，故为低 |
| L2 | `src/api.rs:858`、`docs/templates/daily-note.md` | 快速添加日程写入 `## 今日日程`，内置日记模板是 `## 日程`；同一天可能产生两个日程段，解析能工作但结构不统一 |
| L3 | `src/index.rs:217-235` | 日记路径只校验日期形状，`2026/02/2026-02-31.md` 也会被当日记；日程视图不会访问该非法日期 |
| L4 | `src/schedule.rs:74-95` | `- 10:00 #工作` 在去掉标签后标题为空，但没有重新判定为未识别，会产出空标题日程 |

## 4. 关键问题详述

### H1：在途保存会把后续输入误标为“已保存”

位置：`web/app.js:967-1029`、`web/app.js:1034-1040`、`web/app.js:1848-1884`

触发过程：

1. `flush()` 在 `app.js:975` 取得 `content` 快照并发起 PUT。
2. PUT 未返回时用户继续输入 B；`onChange` 只把 `dirty=true` 并重设定时器（`app.js:1034-1040`）。
3. 原 PUT 返回后 `app.js:983-985` 无条件执行 `dirty=false`、显示“已保存”；磁盘上只有 A，没有 B。
4. 轮询看到 revision 变化后调用 `refreshFromDisk()`；`app.js:1875` 只检查一次 dirty，随后 GET 返回 A，`app.js:1879-1880` 用 A 重载编辑器，B 消失。

现有 `saving/queued` 只保证两个请求不并发，不等于“保存的是最新内容”。修复需要给每次编辑分配单调递增的 generation，响应只有在该 generation 仍是最新、且编辑器内容仍等于请求快照时才能清 dirty；否则必须继续排一次保存。

### H2：导航没有等待真实保存，失败也继续销毁编辑器

位置：`web/app.js:970-972`、`web/app.js:987-1026`、`web/app.js:1630-1651`、`web/app.js:1265-1275`

`flush()` 在 `saving` 为真时只把 `queued=true` 然后立即返回；`route()` 在 `app.js:1633-1638` 先清掉 `activeFlush/activeDirty`，再继续路由。即使保存返回 401、409、500，`flush()` 也只在页面里显示状态栏/冲突条，不会向上抛出阻止导航。随后 `teardownEditor()` 清掉 `saveTimer` 并销毁 CodeMirror，未落盘内容随编辑器一起消失。

修复应让路由等待一个“包含排队尾部”的 Promise；保存失败或仍处于冲突状态时禁止离开，或明确弹出“放弃/重试”选择，不能静默继续。

### H3：勾选待办会清除整篇未保存内容的 dirty

位置：`web/src/editor.js:254-263`、`web/app.js:1068-1078`

`toggleTask()` 先改 CodeMirror 文档，触发 `onChange` 和 `scheduleSave()`；随后 `onToggleTodo` 只发送单行 PATCH。后端 `patch_todo`（`src/api.rs:524-543`）确实只回写定位到的那一行，不会保存其他正文。但前端在 PATCH 成功后执行 `dirty=false`、`clearTimeout(saveTimer)`，把“其他正文未保存”这个事实抹掉了。

修复：把待办回写和整文件保存分成两个状态；单行回写成功后只更新该行相关的行号/版本，不应改变整篇 `dirty`，也不应取消待保存的整篇 saveTimer。

### H4：BOM/CRLF 文件版本号口径不一致

位置：`src/api.rs:268-271`、`src/api.rs:312-314`、`src/api.rs:342`、`src/index.rs:170-184`

- `read_file` 先剥 BOM、把 CRLF 归一化成 LF，再用归一化内容算 version。
- `save_file` 读取磁盘原文算 current_version，原文含 BOM/CRLF，与 GET 返回的 version 不同。
- 保存成功后返回的 version 用 `body.content`（LF、无 BOM）计算，而 `index.rs` 用磁盘原文（含 BOM/CRLF）计算。

实测结果：BOM 文件内容不变直接 PUT 返回 409；纯 CRLF 文件第一次保存 200，第二次用返回的 version 保存相同内容返回 409。只要用户打开过 BOM 文件，编辑器就无法正常保存；CRLF 文件第一次保存后下一次保存必定冲突。

修复应统一版本输入：定义“版本计算字节”为原始磁盘字节（或明文 canonical bytes），GET、PUT、PATCH、索引全部使用同一函数；优先保留原 BOM/EOL，写回后再从磁盘读取一次计算返回值。

### H5：旧布局下 `.trash` 会被重新扫描

位置：`src/api.rs:363-371`、`src/index.rs:44-60`、`src/index.rs:286-290`、`src/watch.rs:69`

`delete_file` 把文件重命名为 `<root>/.trash/<时间戳>-<名字>`。`architecture.md:34` 明确 `.trash` 不扫描，但 `Ignored::load` 的默认目录只有 `.git`，`Index::scan` 也没有排除 `.trash`。旧布局（没有 `notes/` 子目录）时 `notes_root == root`，于是 `.trash` 直接进入扫描范围。

实测：删除 `a.md` 后重建索引，文件列表出现 `.trash/20261101-210014-a.md`，待办总数仍为 1。监听路径 `watch.rs:69` 也只检查 `.md` 和 `.git`，运行中也可能把移动进 `.trash` 的文件重新收录。

修复：把回收站路径加入统一的忽略规则（扫描和监听共用），并把删除事件作为“从索引移除”处理，而不是只处理仍然存在的文件。

### H6：监听与扫描的准入规则不一致

位置：`src/watch.rs:62`、`src/watch.rs:69-79`、`src/index.rs:96-100`

`watch.rs:62` 的 `!path.is_file()` 会在删除/重命名事件上直接 `continue`，因此永远不会调用 `index.upsert` 的“不存在则 retain 删除”分支。外部删除文件后，搜索、待办、标签、反向链接仍保留幽灵记录，直到手动重建索引或重启。

另一个问题是监听只检查 `.md` 和 `.git`，没有使用 `Ignored`。`.gitignore` 里写了 `drafts/`，外部创建 `drafts/x.md` 后，扫描不收录，但监听会把它重新加入索引。PRD 4.6 要求扫描、搜索、监听使用同一套忽略规则。

修复：监听收到 `Remove`/`Rename` 时构造相对路径并调用统一的 `remove_from_index`；对所有新增/修改文件走与 `Index::scan` 相同的 `Ignored::is_ignored` 检查。`Ignored::load` 的 warnings 还应合并到 `Index.warnings`。

### H7：标准 `notes/` 布局下 Git 恢复路径拼接错误

位置：`web/app.js:1799-1826`、`src/api.rs:1120-1153`

历史页从 `git show --patch` 的 diff 中提取 `+++ b/<path>`，在标准布局下是 `notes/raw/a.md`（仓库相对）。点击“恢复”后，前端原样把 `notes/raw/a.md` 发给后端；后端 `st.resolve(&body.path)` 以 `notes_root`（`<root>/notes`）为基准，实际检查的是 `<root>/notes/notes/raw/a.md`，于是 `git show <hash>:notes/raw/a.md` 虽然能取到文件，但 `st.resolve` 已把目标路径拼错，最后返回 404。

已用 `.demo/review-git-restore` 标准布局实测：提交历史存在且文件在提交里，恢复请求返回 `404 not_found`，当前内容保持 `current`。只要使用推荐的 `root/notes` 布局，按文件恢复功能就是坏的。

修复：前端提交给恢复接口的路径应是“notes 根相对路径”；后端也应明确校验路径位于 `notes_root` 且位于 git 仓库内，并在 UI 中显示可读错误。最好增加一条真实 Git 仓库的标准布局测试。

### H8：语料生成脚本可删除任意传入目录

位置：`tools/gen-corpus.js:6-16`、`tools/legacy-corpus.js:8-15`

两个脚本都直接用 `process.argv[2]` 作为删除根：

```js
const root = process.argv[2];
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(root, { recursive: true });
```

没有检查路径是否位于仓库 `.demo`、是否存在所有权标记、是否是空目录，也没有二次确认。误把 `D:\MyNotes` 或仓库根传进去，会先递归删除真实内容再写入测试语料。即使用户按文档“只传测试目录”，一次手误就是不可逆数据损失。

修复建议：限制输出根必须在仓库 `.demo/` 下且以显式标记（如 `.noteschain-fixture`）标识；删除前要求 `--force` 加交互确认；或永远创建唯一临时目录而不删除既有目录。

### H9：发布闸门可以对未执行的验证报“全部通过”

位置：`tools/release-gate.ps1:16-30`、`tools/release-gate.ps1:86-116`、`tools/browser-probe.ps1:427-435`

问题分三层：

1. 构建检查只判断 `target/release/noteschain.exe` 存在，不检查 `cargo build` 退出码。编译失败但旧 exe 仍在时，后续所有测试都会针对旧二进制运行。
2. `cargo test` 只匹配第一个 `test result: ok`，不检查退出码和全部测试目标；`clippy` 只数输出文本，不检查退出码。
3. 浏览器层只遍历 `browser-probe.ps1` 输出中的 `=>` 记录并 Record；若 Edge 缺失、CDP 异常、脚本在输出结果前终止，浏览器没有任何记录，但 `$results` 也没有 FAIL，最终仍 `exit 0`。`browser-probe.ps1` 自身也不返回非零退出码。

本轮实测正好触发了第 3 层：发布闸门退出码 0，汇总只有“全部通过（6 项）”，其中没有一条浏览器场景；这不能证明浏览器交互通过。

修复：每一层都检查 `$LASTEXITCODE`/`$?`；构建前删除或校验 exe 时间戳；浏览器探针必须返回结构化结果和非零失败码；断言必须有场景级不变量，而不是只排除空字符串和 `no-`。测试数量也不要写死在报告/文档里，改为断言“执行层数和必需场景集合”。

## 5. 测试覆盖缺口

当前测试主要覆盖解析和纯函数，缺少这几类“数据安全行为”测试：

- BOM、CRLF、混合 EOL 文件的 GET → PUT → GET 往返，包含 version 连续两次保存。
- 编辑中慢保存、保存失败后导航、勾选待办与整篇未保存内容混用。
- 标准 `notes/` 布局的 Git commit → log → show → restore 全链路。
- 外部删除、外部重命名、`.gitignore` 文件被监听重新收录。
- 旧布局下 `.trash` 不进入文件列表、待办、标签、搜索。
- `.demo` 语料生成器误传真实目录的防护。
- 发布闸门在构建失败、浏览器探针失败、零断言时必须失败。

这些测试应优先用真实启动进程的接口测试，而不是只测纯函数；尤其是 H1/H2/H3 必须覆盖“请求在途时继续输入”的时序，否则现有顺序场景永远测不出来。

## 6. 修复优先级建议

### 第一批：真实笔记目录放行前必须修

1. 统一版本计算（H4），并补 BOM/CRLF 往返测试。
2. 修复 `.trash` 与监听删除/忽略规则（H5/H6），保证删除后不复活、不残留幽灵记录。
3. 重做前端保存状态机（H1/H2/H3），确保“已保存”只在最新输入真正落盘后显示。
4. 修复 Git 标准布局恢复路径（H7），补真实 Git 集成测试。
5. 给语料生成器加删除保护（H8）。

### 第二批：发布前修

1. 发布闸门（H9）改为退出码和场景集合驱动，禁止零断言通过。
2. 所有写接口统一锁键与 `.md` 校验，创建接口锁内二次检查（M2/M3/M6）。
3. Git 自动提交只提交笔记目录路径（M7）。
4. 给路径解析补软链接/缺失父目录防护（M5）。
5. 修复混合 EOL 行回写（M1）。

### 第三批：体验与可维护性

1. 给异步路由、轮询、待办筛选加请求代际/目标态校验（M8）。
2. 加卸载未保存提示（M9）。
3. 合并 `.gitignore` warnings（M10），修复测试断言过宽（M11）。
4. 统一日程标题和日期合法性校验（L2/L3/L4）。

## 7. 已确认做得好的地方

- 路径穿越的基础防护在 `state.rs::resolve` 中已考虑绝对路径、`..`、根目录包含校验，测试覆盖了三种常见输入。
- 原子写使用同目录临时文件 + rename，符合 PRD 5.3 的基本方向。
- 待办三级定位有单元测试，包含外部插行、重复文本并列拒绝、代码块内待办排除。
- 前端 Markdown 渲染的主要路径先 `esc` 再拼 HTML；没有发现可成立的 XSS 绕过。
- API 与静态资源统一走 Cookie 鉴权，token 随机 32 字节、`HttpOnly`、`SameSite=Strict`，服务只绑回环地址。
- `cargo test`、`cargo clippy`、语法检查和当前前端产物同步均通过，说明项目的基本卫生不错；问题集中在跨层状态与验证链路，而不是编译质量。

## 8. 审查范围与声明

本报告只做 review，不包含业务代码修复、提交、推送或发布操作。运行验证只使用了 `.demo/review-*` 临时目录和现有生成的 `.demo` 测试语料；没有写入任何真实笔记目录。若要按本报告修复，建议按第 6 节拆成独立提交，每个高风险项都先补失败测试再改实现。

## 9. 修复状态（2026-11-01 当晚）

| 编号 | 状态 | 处理 |
| --- | --- | --- |
| H1 | ✅ | 保存状态机改为「以磁盘内容 lastSaved 为基准」判断脏；保存期间的新输入会继续入链保存，不再被误标已保存 |
| H2 | ✅ | 所有保存串成一条 Promise 链；导航前 `await` 整条链，冲突/失败时留在当前页不销毁编辑器 |
| H3 | ✅ | 编辑器勾待办前先 `flush` 正文；PATCH 后把编辑器对齐到磁盘内容，不再整篇清 dirty |
| H4 | ✅ | 新增 `index::canonical_text` / `version_of_text`：版本号统一按「去 BOM + CRLF→LF」计算，GET/PUT/PATCH/索引同一口径。实测 BOM 首次保存 200、CRLF 连续两次保存 200/200 且磁盘保持 CRLF |
| H5 | ✅ | `Ignored` 默认排除 `.trash`；旧布局实测删除后重建索引列表为空、待办 0 |
| H7 | ✅ | `git restore` 接受仓库相对路径，自动剥 `notes/` 前缀；实测 `notes/raw/a.md` 与 `raw/a.md` 均 200 |
| H9 | ✅ | 闸门：构建按退出码判定；浏览器层零断言直接判 FAIL |
| M1 | ✅ | 待办回写按行保留各自行尾；混合 EOL 实测只有目标行变化（1 CRLF / 4 LF 保持不变） |
| M2/M3/M6 | ✅ | 写接口统一 `.md` 校验、锁键统一用 notes 相对路径、创建接口重名检查移入锁内（非法扩展名实测 400） |
| M5 | ✅ | 路径解析向上找最近已存在祖先做 canonicalize，再拼回剩余层级 |
| M7 | ✅ | `git commit` 带 pathspec（仅限笔记目录），不再连带父仓库已暂存内容 |
| L2 | ✅ | 日程段标题优先写 `## 日程`，老文件 `## 今日日程` 也能识别并复用 |
| H4 | ✅ | 追加保留原 BOM：save_file 检测原文件有 BOM 时写回时补回 |
| H5 | ✅ | 监听与扫描共用 Ignored：跳过 `.git`/`.trash`/`.gitignore` 命中项 |
| H6 | ✅ | 监听收到删除/重命名时从索引移除（Index::remove）；增量也应用 .gitignore |
| H8 | ✅ | `gen-corpus.js`/`legacy-corpus.js` 增加护栏：目标目录非空且无 `.noteschain-corpus` 标记则拒绝执行（exit 2） |
| H9 | ✅ | gate 的 test/clippy 改按 `$LASTEXITCODE` 判定；browser-probe 有 `no-*`/`JS-ERROR` 时 exit 1 |
| H2 | ✅ | route 只在 flush 成功后才注销 activeFlush/activeDirty，冲突后再次导航仍受保护 |
| H3 | ✅ | 根因在 `toggleTask` 先 dispatch 再回调：改为有回调时不改本地文档，等 PATCH+刷新；Playwright 实测 0→1→0 |
| 其余 M4/M8~M11、L1/L3/L4 | ⬜ | 未处理（体验类/低风险），留待后续 |

### 两轴评审（code-review skill，2026-11-01 晚）

- Standards：需返工的硬性问题（提交信息格式、写序、重复的 `.md` 校验）已处理：写序改为 upsert 先于 notify/revision，`.md` 校验抽成 `ensure_markdown`。提交信息格式为历史提交，后续遵守。
- Spec：指出 H5/H6/H8/H9 半做或缺失 → 已按上表补齐。

### 复审跟进（同日）

- **H1（复审发现 P0 死锁）**：原实现 `doSave` 完成后 `return flush(false)`，会把新 `.then` 追加到**包含自己**的同一条链上，Promise 永不结算。已改为 `doSave` 内部 `for` 循环直到 `!isDirty()`，不再回调 `flush()`。
- **H7（复审发现嵌套仓库失效）**：原来只剥 `notes/`，未剥 `repo_root→root` 前缀。现在按「仓库相对 / 项目根相对 / notes 相对」生成候选路径，逐个 `git show` 试。实测嵌套仓库 `.demo/nest`（repo 在上一层）恢复 `inner/notes/raw/a.md` → 200。
- **补充**：`onToggleTodo` 现在检查 `flush` 返回值，冲突时不继续 PATCH；`release-gate` 增加「探针非零退出即 FAIL」。
