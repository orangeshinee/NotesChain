# NotesChain 用户反馈整改计划（2026-11-02）

> 本文是**整改计划**，不是实现。目的是先把 15 条反馈整理清楚、定位根因、排出优先级，请你确认「写得对不对、优先级是否认可、几个含糊点怎么定」，确认后再动代码。
>
> 现状基线：`master`、纸感主题 v0.6-7。全部改动遵守 AGENTS.md 红线：**绝不改写已有笔记文件**；改 `web/src/editor.js` 必须先 `cd web; npm run build`；提交前 `pwsh -File tools/release-gate.ps1` 全绿。

---

## 〇、已确认决策（2026-11-02，用户回复）

| 编号 | 决策 |
| --- | --- |
| 1 | ✅ 确认：标签页摘要是 frontmatter 的 `---`，改为显示标签所在行 |
| 7 | ✅ 确认：形如 `- ```sql`（列表项上直接跟围栏）无法渲染，按此修 |
| 8 | ✅ 确认：「进度条回顶部」= 页面滚动位置 |
| 10 | ✅ 确认：「地服」= diff |
| 12a | ✅ 确认：启动加 `--no-token` 开关（默认仍校验 token） |
| 13 | ⚠️ 缩小范围：**不加新筛选项**，只把现有「标签筛选 + 排序」两个控件**靠右**（现在居中分布） |
| 14 | ✅ Git 提交默认间隔改 **5 分钟**，且做成**可配置** |
| 15.2 | ⏸ 模板功能往后放，先解决前面的问题 |
| 15.4 | ⏸ 预览/编辑样式整体调整往后放 |

> 因此本计划的实际执行范围：先做 P0（1、6、2）→ P1（4、9、8、7、5、3）→ 其余 P2 中 13（仅靠右）、14（5 分钟可配）、12a（--no-token）、10、15.3、11、12b、15.1；15.2、15.4 暂缓。

---

## 一、优先级定义

| 级别 | 含义 |
| --- | --- |
| **P0** | 数据/正确性问题，或「默认视图里核心动作直接失效」，先修 |
| **P1** | 高频使用体验，或小改动大收益，紧接着修 |
| **P2** | 功能增强 / 交互优化，做了明显更好 |
| **P3** | 探讨项或整体大改，需先设计再动手 |

## 二、总览（按优先级排序）

| 优先级 | 编号 | 问题（你的原话缩写） | 类型 | 主要落点 | 体量 |
| --- | --- | --- | --- | --- | --- |
| **P0** | 1 | 点标签后结果全是「杠杠杠」，应显示标签所在行 | Bug·正确性 | `api.rs::search` / `first_line`、`app.js::showTag` | S |
| **P0** | 6 | 元数据 `tags` 没进左侧标签树 | Bug·数据 | `markdown.rs::collect_tags`、`index.rs::load_file` | M |
| **P0** | 2 | 预览里待办不能勾选，样式还不如编辑 | Bug·核心动作 | `app.js::renderPreviewNodes/renderList`、`style.css` | M~L |
| **P1** | 4 | 日记里 `!p1` 优先级没高亮，只是纯文本 | 一致性问题 | `app.js::inline`、`style.css .prio` | S |
| **P1** | 9 | 首页待办里行内代码没渲染，是纯文本 | Bug·渲染 | `app.js::appendTodoText` | S |
| **P1** | 8 | 勾选首页待办后自动保存刷新，页面滚回顶部 | Bug·体验 | `app.js::showHome/toggleTodo/refreshFromDisk` | S~M |
| **P1** | 7 | 代码块前面有「杠」就渲染不出 | Bug·渲染 | `app.js::renderMarkdown/parseList`、`markdown.rs::code_block_flags` | M |
| **P1** | 5 | PDF 导出没用、看不懂，隐藏它 | 精简 | `app.js` exportBtn、`style.css` 打印段 | S |
| **P1** | 3 | 预览双击进编辑，保存后回到预览 | 交互增强 | `app.js::setPreview/showFile` | S~M |
| **P2** | 13 | 首页待办加筛选（标签/排序），控件全部靠右 | 增强·布局 | `app.js::buildTodoSection`、`style.css .section-head` | M |
| **P2** | 14 | 自动保存/刷新/commit 时基不透明，commit 是否太频繁 | 行为/配置 | `git.rs` 防抖、`app.js` 各定时器、config | S~M |
| **P2** | 12a | 本地（127.0.0.1）免 token 限制 | 安全/配置 | `main.rs` 参数、`api.rs::auth/root` | S~M |
| **P2** | 10 | 历史提交点开直接铺开 diff，不直观 | UX | `app.js::showCommit`、`style.css .patch` | M |
| **P2** | 15.3 | 首页「新增日程」加时间选择 | 增强 | `app.js::buildSchedule`、`schedule.rs` | M |
| **P2** | 11 | 需要一份接口文档 | 文档交付 | 新增 `docs/api.md` | M |
| **P2** | 12b | 编辑时输入 `#T1` 联想出 `test a` | 增强 | `editor.js` + `app.js` + 依赖 | M |
| **P2** | 15.1 | 配置界面：字体 / 字号 / 主题 | 新功能 | `index.html` + `app.js` + `style.css` | M~L |
| **P2** | 15.2 | 模板管理：可修改模板、建文件时选模板 | 新功能 | `api.rs` 模板接口、`app.js` 新建弹窗 | L |
| **P3** | 15.4 | 预览/编辑样式「比较怪异」 | 设计·大改 | `style.css`、`editor.js` theme | L |

**建议分批**：
- **第一批（P0，3 项）**：1、6、2 —— 修完默认视图就能正常用。
- **第二批（P1，6 项）**：4、9、8、7、5、3 —— 打磨渲染与编辑闭环。
- **第三批（P2，逐个独立）**：13、14、12a、10、15.3、11（文档可随时并行）；随后 12b、15.1、15.2。
- **第四批（P3）**：15.4 先出设计稿/截图确认，再动。

---

## 三、逐条详解

### 1）标签页渲染错误（P0）

- **现象**：点标签进入标签页，每条结果的摘要都是「杠杠杠」（即 `---`），看不到内容；应显示「该标签所在那一行」。
- **根因（已定位）**：点标签时前端 `showTag()` 调 `GET /api/search?tag=...`（无关键词）。后端 `search()` 在无关键词时用 `first_line(&file.content)` 作为摘要（`src/api.rs:1567-1579`），而 `first_line()`（`src/api.rs:1632`）取「第一个非空行」。新笔记都以 YAML frontmatter 开头，第一行正是 `---`，于是所有条目摘要都成了 `---`。
- **拟改法**：
  1. 后端：无关键词的标签搜索改为返回**命中标签的那一行**（遍历 `file.content` 找含该标签的行，返回其行号与摘要）；对 frontmatter 里的标签，返回 frontmatter 中 `tags:` 所在行。`first_line()` 增加「跳过 frontmatter`---`」兜底。
  2. 前端 `showTag()`（`web/app.js:1416`）：结果项显示行号 + 该行文本，点击可跳到对应行。
- **涉及文件**：`src/api.rs`（`search`、`first_line`，新增「找标签行」helper）、`web/app.js`（`showTag`）。
- **验证**：`smoke.ps1` 加一条「标签搜索摘要不是 `---`」断言；浏览器点任意标签看结果。

### 2）预览待办不能勾选，样式不如编辑（P0）

- **现象**：默认进入的是**预览**（只读），预览里的待办是 `☐/☑` 文本符号，点不动；只有切到编辑态（CodeMirror）才能勾。
- **根因（已定位）**：
  - 预览用 `renderMarkdown` → `renderList`（`web/app.js:279-300`）把任务渲染成 `<span class="box">☐</span>`，纯文本，无交互。
  - 历史原因：`showFile` 里注释（`web/app.js:923-924`）写明——曾在预览里勾选后对**隐藏的** CodeMirror `setValue`，触发测量死循环、整页卡死，因此回退为只读。
  - 样式：`.doc li.task .box` 是灰色字符；编辑态是真正的 `<input type=checkbox class="cm-todo-box">`，所以「预览样式不如编辑」。
- **拟改法**：
  1. `renderList` 的任务行改为渲染**真实 `<input type=checkbox class="preview-todo">`**（`.preview-todo` 样式已存在，`style.css:642`），并带上 `data-line / data-path`。
  2. `renderPreviewNodes(content, filePath)` 打开时已知 `version`，给每个复选框挂 `change` → 直接 `PATCH /api/todos`（`{path,line,raw_line,version,done}`，raw_line 由原始行 `stripDoneMarker` 得到）；成功后用返回的新 `version`、重新拉取文件内容并**只重渲染预览**。
  3. **不再对隐藏的 CM `setValue`**：给编辑器打 `previewDirty` 标记；当用户从预览切到编辑时，先 `host.hidden=false` 再重建/覆盖编辑器内容（避免 display:none 测量死循环），或直接 teardown 后重建。
- **涉及文件**：`web/app.js`（`renderList`、`renderPreviewNodes`、`setPreview`、`showFile`）、`web/style.css`（`.preview-todo` 已具备，微调）。
- **风险**：这里踩过坑（pitfalls #4 隐藏 CM setValue 死循环），必须按「预览态与编辑器解耦」来改；改完必须跑 `browser-probe` 勾选场景 + `e2e-checkbox.ps1`。

### 3）预览双击进编辑、保存回预览（P1）

- **现象**：希望预览时双击进入编辑，保存后又自动回到预览。
- **拟改法**：在预览 `<article class="doc preview">` 上监听 `dblclick` → `setPreview(false)`；记录「本次是从预览进入编辑」的标志，`doSave()` 成功后若该标志为真则 `setPreview(true)`。与表格双击（widget 内 `dblclick`）不冲突（表格双击是进源码，另算）。
- **涉及文件**：`web/app.js`（`setPreview`、`showFile` 的 `doSave`）。
- **验证**：`browser-probe` 新增「双击预览→编辑→保存→回预览」场景。

### 4）日记待办优先级未高亮（P1）

- **现象**：日记正文里 `!p1`（P1）是纯文本，没渲染成徽标；首页待办里已有 `.prio` 徽标，希望日记里也一致。
- **根因（已定位）**：首页待办走 `makeRow` 单独渲染 `!pN`（`web/app.js:803-805` + `.prio` 样式 `style.css:859`）；但正文预览走 `inline()`（`web/app.js:70-93`），完全没处理 `!pN`。
- **拟改法**：在 `inline()` 增加 `!p[0-9]` 识别，边界规则与后端 `extract_priority`（`src/markdown.rs:27`）保持一致（前后必须是空白/行首行尾），输出 `<span class="prio pN">PN</span>`，直接复用现有 `.prio` 配色。
- **涉及文件**：`web/app.js::inline`（顺带 `editor.js::inlineHtml` 可同步，保持编辑态表格一致）。
- **验证**：单测不涉及；浏览器看含 `!p1` 的日记预览 + 表格单元格。

### 5）隐藏 PDF 导出（P1｜S）

- **现状**：`showFile` 里有「导出 PDF」按钮（`web/app.js:946-958`），实现是「切渲染视图 + `window.print()`」，确实不是真正的 PDF 导出，容易误解。
- **拟改法**：移除该按钮与相关逻辑（保留打印 CSS 不影响，或一并清理）。如需保留能力，可改成「打印」并加明确说明，但按你的要求**直接隐藏**。
- **涉及文件**：`web/app.js`（删 exportBtn）；可选清理 `style.css` 打印段。

### 6）元数据 tags 未进标签树（P0）

- **现象**：`frontmatter` 里 `tags: [工作, 生活]` 的标签，左侧标签树看不到。
- **根因（已定位）**：`collect_tags()`（`src/markdown.rs:287`）只按正文逐行找 `#标签`；`extract_tags()` 还会跳过 `#` 开头行，而 frontmatter 的 `tags:` 行根本不含 `#`，所以完全没被收集。`index.rs::load_file`（`src/index.rs:187`）用的就是这个 `collect_tags`。
- **拟改法**：
  1. 在 `markdown.rs` 增加 `parse_frontmatter_tags(content)`：支持 `tags: [a, b]`（flow 列表）、`tags: [a]`、`tags:` 换行 `  - a`（块列表）、`tags: a` 单值；空 `[]` 返回空。
  2. `index.rs::load_file` 把 frontmatter tags 合并进 `file.tags`（去重）。这样 `list_tags`（`src/api.rs:1503`）自动生效，标签树、搜索、计数都跟着对。
  3. 前端 `renderMetaCard()` 已有 tags 渲染（`web/app.js:118-123`），保持不变。
  4. 注意：`collect_tags` 仍要统计正文 `#标签`，两者是并集。
- **涉及文件**：`src/markdown.rs`（新增解析 + 单测）、`src/index.rs`（合并）。
- **验证**：`cargo test` 新增 frontmatter tags 用例；浏览器新建带 tags 的笔记看标签树。

### 7）代码块前有「杠」无法渲染（P1）

- **现象**：代码块处在列表项内（前面有列表横杠 `-`、缩进较深）时渲染不出来。
- **根因（推断）**：
  - 预览 `renderMarkdown` 的围栏识别正则只允许 **≤3 个空格** 缩进（`^\s{0,3}(`{3,}|~{3,})`，`web/app.js:143`）；列表内代码块通常缩进 ≥4 空格，于是识别不到围栏，被当普通段落/列表。
  - `parseList()`（`web/app.js:245`）按行吃列表项，遇到围栏行会提前截断，列表内的代码块结构丢失。
  - 后端 `code_block_flags()`（`src/markdown.rs:55`）同样限制 `indent <= 3`，可能导致「代码块里的 `- [ ]` 被误当作待办」。
- **拟改法**：预览与后端都放宽围栏缩进判定（按「列表上下文相对缩进」处理），并在 `parseList`/`renderList` 中识别列表项内的围栏代码块，按 `<pre><code>` 渲染。补一个含「列表内代码块」的渲染用例。
- **涉及文件**：`web/app.js`（`renderMarkdown`/`parseList`）、`src/markdown.rs::code_block_flags`（+单测）、`web/src/editor.js`（确认 `buildDecorations` 不破坏，必要时同步）。
- **待确认**：见第六节第 2 条——请给一个最小复现片段（到底是「列表内缩进代码块」还是别的「杠」）。

### 8）首页勾选后刷新导致滚回顶部（P1）

- **现象**：首页（尤其上方有无序列表/日程时）勾选一条待办，页面自动保存刷新，滚动位置回到顶部，体验差。
- **根因（已定位）**：
  - `toggleTodo()`（`web/app.js:856`）本身只删本行、改计数；但后端 `PATCH` 会 `revision+1`，5 秒轮询 `pollRevision`→`refreshFromDisk`（`web/app.js:1885/1901`）发现首页无 `activePath` 就 `showHome()` 整页重渲。
  - `showHome()`（`web/app.js:375`）开头有 `const keepScroll = main.scrollTop;` **却从未使用**，所以整页重渲后滚动归零。
- **拟改法**：
  1. `showHome()` 渲染完成后恢复 `main.scrollTop`（用 rAF 两帧，跟 `refreshTodoSection` 的做法一致）。
  2. 更彻底：勾选成功后就地删行、更新计数，并**吞掉自己触发的这次 revision 刷新**（记录「刚由本页改动」的短窗口，轮询命中则跳过重渲），避免无谓整页重排。
  3. `refreshTodoSection()` 已有滚动保持逻辑，可复用。
- **涉及文件**：`web/app.js`（`showHome`、`toggleTodo`、`pollRevision`/`refreshFromDisk`）。
- **待确认**：第 3 条——「进度条」是否就是页面滚动位置（我按此理解）。

### 9）首页待办行内代码未渲染（P1）

- **现象**：首页待办事项里的 `` `code` `` 显示为带反引号的纯文本。
- **根因（已定位）**：`appendTodoText()`（`web/app.js:734-750`）只处理 `#标签`，其余按纯文本 `createTextNode`，不解析反引号。
- **拟改法**：`appendTodoText()` 增加行内代码解析（`` `...` `` → `<code>`），与标签识别一起做一遍分词，保证二者都能渲染、且标签仍可点击。
- **涉及文件**：`web/app.js::appendTodoText`（可能抽出一个「待办富文本」公共函数，正文 `inline()` 已有类似逻辑可参考）。

### 10）历史提交记录点击展开 diff 不直观（P2）

- **现象**：「历史」里点一条提交，直接铺开一大段原始 diff（`<pre class="patch">` 文本），不好看也不好读。你问「具体改哪些地方」。
- **根因（已定位）**：`showCommit()`（`web/app.js:1836-1880`）把 `data.patch` 原样塞进一个 `<pre>`；`/api/git/show` 返回的是整段 unified diff 字符串。
- **拟改法**：
  1. 前端把 patch **按文件切块**（`diff --git a/x b/x` 分隔），每个文件一个可折叠卡片：文件头（路径 + 增删行数）+ 恢复按钮；展开看该文件 diff。
  2. diff 行做最小着色：`+` 绿底、`-` 红底、`@@` 灰；默认折叠全部、只显示文件清单概览。
  3. 后端 `/api/git/show` 可保持不变（继续给原始 patch，前端解析）；也可新增结构化文件数组（更稳，推荐）。
  4. 样式在 `style.css` 新增 `.commit-file / .diff-add / .diff-del / .diff-hunk`（现有 `.patch` 样式 `style.css:~973`）。
- **涉及文件**：`web/app.js::showCommit`、`web/style.css`；可选 `src/api.rs::git_show` 增结构化返回。
- **待确认**：第 4 条——你写的「地服」我理解为 **diff**。

### 11）接口文档（P2）

- **现状**：仓库没有单独的接口文档；接口清单散落在 `src/api.rs::router`（`src/api.rs:55-88`）与 PRD 7.3。
- **拟交付**：新增 `docs/api.md`，覆盖：鉴权方式（Cookie/token）、统一响应/错误码、每个接口的方法/路径/参数/请求体/响应体/示例（以 `src/api.rs` 为准，含 `/api/todos/append`、`/api/notes/new`、`/api/templates`、`/api/schedule/append`、`/api/git/*` 等 PRD 未列的现有接口）。**以代码为准**核对，而不是照抄 PRD。
- **涉及文件**：新增 `docs/api.md`（可能同步 `docs/handover.md` 导航）。

### 12）Token 本地放开 + 标签联想（P2）

**12a 本地免 token**
- **现象**：你不清楚 token 含义，希望本机（127.0.0.1）使用时不要 token 限制。
- **说明（实话）**：token 是防「同机其它进程/共享云桌面上的其他用户」直接调本机端口的 API 读写笔记（PRD 9.1 明确：只绑 127.0.0.1 ≠ 安全）。默认关掉会降低共享机器的安全性。
- **拟改法（推荐）**：**默认保持开启**，新增开关 `--no-token`（或设置项），启动时显著打印「本次未启用 token，仅建议在个人独占机器使用」。也可加 `--token-file` 之类持久化（可选）。**不建议**默认关闭。
- **涉及文件**：`src/main.rs`（参数）、`src/api.rs`（`auth`、`root` 放行逻辑）、`src/state.rs`（token 生成可空）。
- **待确认**：第 5 条——你说的「cos」是否指 `localhost`？以及是否接受「默认仍开、加开关」这一安全折中。

**12b 标签联想**
- **现象**：编辑时输入 `#T1` 希望弹出 `test a` 供选择。
- **拟改法**：给 CodeMirror 加补全：引入 `@codemirror/autocomplete`，写一个 completion source：在 `#` 后取当前前缀，从「全部标签列表」里做前缀/子串匹配，回车选中补全整段标签。标签列表来自 `/api/tags`（`flattenTags` 已有，`web/app.js:573`），启动/刷新时传入 `makeEditor`。
- **涉及文件**：`web/package.json`（加依赖）、`web/src/editor.js`（completion 扩展）、`web/app.js`（把标签列表传进 `makeEditor`）。**改完必须 `npm run build`**。
- **验证**：`browser-probe` 新增「输入 `#` 触发联想」场景。

### 13）首页待办筛选 + 控件靠右（P2）

- **现状**：其实已有「全部标签」下拉 + 「默认/按优先级」排序 + 「未完成/已完成/全部」切换（`buildTodoSection`，`web/app.js:582-669`）。你说「增加筛选、标签筛选、排序筛选、对齐靠右」，我理解是：**这些控件要更完整、更显眼、统一靠右**。
- **拟改法**：
  1. 把控制区合并成一条右侧工具条（`.section-head` 里加一个 `.todo-controls`，`margin-left:auto`），标题与计数在左，筛选/排序/状态在右。
  2. 在现有基础上**补**：优先级筛选（P0–P9/无）、来源筛选（日记/主题笔记/某文件）、可选关键词框。
  3. 统一控件高度与间距，修掉换行错位。
- **涉及文件**：`web/app.js::buildTodoSection`、`web/style.css`（`.section-head`、`.todo-filter`、新增 `.todo-controls`）。
- **待确认**：第 6 条——除标签/排序外，你还要哪些筛选项？「全部靠右」确认是整组控件靠右。

### 14）自动保存/刷新/commit 的时基与频率（P2）

- **现状（透明化）**：
  - 编辑自动保存：**800ms** 防抖（`web/app.js:1065`）。
  - 外部改动轮询：**5 秒**（`web/app.js:1898`）。
  - 文件监听：**300ms** 防抖（`src/watch.rs`）。
  - Git 自动提交：**停止变更 30 秒**后合并提交一次；连续编辑超 **5 分钟**强制提交（`src/git.rs:13-14`）。
- **是否太频繁**：30 秒防抖下，「改一下停一下」的一天可能几十个提交。建议：把 Git 防抖做成**可配置**（默认调大，如 2~5 分钟），并在设置界面/`/api/config` 暴露；自动保存与轮询一般无需改。
- **拟改法**：`git.rs` 的 `DEBOUNCE/FORCE_AFTER` 从常量改为可读配置；提供设置项或在 `--dir` 同级配置；状态区继续显示「上次提交/待提交」。
- **涉及文件**：`src/git.rs`、`src/main.rs`/`src/state.rs`（配置注入）、`web/app.js`（设置展示）。
- **待确认**：第 8 条——你期望的提交间隔（2 分钟？5 分钟？仅手动？）还是只做成可配置即可。

### 15.1）配置界面：字体 / 字号 / 主题（P2）

- **现状**：字体、字号、颜色都是 `:root` 变量（`web/style.css:9-74`）；主题只跟随系统 `prefers-color-scheme`，无手动切换；无设置界面。
- **拟改法**：新增「设置」弹层/页面（入口可放侧边栏「⋯」菜单）：
  - 字体：正文字体族（宋/黑/系统）选择 → 写 `--font-read`/`--font-ui`。
  - 字号：基础字号 6 档（对应 `--fs-base`）。
  - 主题：跟随系统 / 浅色 / 深色 → 用 `data-theme` 覆盖 `:root` token（深色已有一份 token，改成可强制切换）。
  - 持久化：存 `localStorage`（纯前端即可，无需后端）；`boot()` 时应用。
- **涉及文件**：`web/index.html`（设置入口/弹层骨架）、`web/app.js`（设置逻辑 + `boot` 应用）、`web/style.css`（`[data-theme]` 覆盖、字体/字号变量）。

### 15.2）模板管理（P2｜L）

- **现状（比你以为的多）**：已支持项目根 `templates/*.md` 自定义模板（`src/api.rs::list_templates` `611`、`create_note` `647`），「＋ 新建」弹窗已能选「日记/普通笔记/自定义模板」并指定日期（`openNewNoteDialog`，`web/app.js:2005`）；但**内置日记/普通笔记模板是编译期常量**（`DAILY_TEMPLATE`/`NOTE_TEMPLATE`），**界面上不能编辑模板**。
- **你要的**：未来能修改模板（日记/会议等），添加文件时选模板生成内容直接填。
- **拟改法**：
  1. 新增模板管理界面：列出模板、可新建/编辑/删除；保存到 `templates/<name>.md`。需要新接口：`GET /api/templates/*name`（读内容）、`PUT /api/templates/*name`（写）、`POST/DELETE`（增删）。
  2. 内置日记/普通笔记模板允许用 `templates/diary.md`、`templates/note.md` 覆盖（没有则用内置常量）；「新建」弹窗里把默认日记/笔记也当模板项展示。
  3. 占位符沿用 `{{TITLE}}/{{DATE}}/{{DATETIME}}/{{TAGS}}`。
- **涉及文件**：`src/api.rs`（模板读写接口、`create_note` 覆盖逻辑）、`web/app.js`（模板管理 UI + 新建弹窗）、`docs/agents/data-conventions.md`（更新约定）。
- **红线**：模板写的是**新建文件内容**，不触碰已有笔记。

### 15.3）首页「新增日程」加时间选择（P2）

- **现状**：首页日程快速添加是一个文本框，必须手写 `15:00-16:00 会议 @地点`（`buildSchedule`，`web/app.js:434-459`；后端 `append_schedule`，`src/api.rs:869`）。
- **拟改法**：改成结构化输入：开始时间 `<input type=time>` + （可选）结束时间 + 标题 + 可选地点/标签；前端拼成 `- HH:mm-HH:mm 标题 @地点` 再 POST。也可以做成「日期 + 时间」选择后写入指定日期的日记（后端 `append_schedule` 现只写今天，需扩展可选日期）。
- **涉及文件**：`web/app.js::buildSchedule`、`src/api.rs::append_schedule`（可选支持 date）、`src/schedule.rs`（确认格式）。

### 15.4）预览/编辑样式整体调整（P3）

- **现象**：你觉得现在预览与编辑样式「比较怪异、不太合理」，但没说具体哪里。
- **拟改法**：这条偏设计，需先对齐再动手。计划：先做一次样式走查（预览 vs 编辑的字号/行高/标题/代码块/表格/待办对齐差异），列出具体问题清单 + 截图对照，确认后再改 `style.css` 与 `editor.js` 的 `theme`。
- **待确认**：第 7 条——请给 1~2 张让你觉得「怪异」的截图或指出具体页面/元素，否则只能做通用收敛。

---

## 四、根因归类小结（便于你判断我理解对不对）

| 类别 | 编号 | 一句话根因 |
| --- | --- | --- |
| 前端渲染器缺能力 | 4、7、9 | `inline()`/`appendTodoText()`/fence 正则没处理 `!pN`、行内代码、列表内代码块 |
| 预览与编辑器耦合的旧债 | 2、3 | 预览只读是因为「隐藏 CM setValue 会卡死」；需解耦 |
| 后端数据/接口口径 | 1、6、10 | 标签摘要取首行（撞 frontmatter）、frontmatter tags 未索引、diff 原样返回 |
| 刷新/滚动 | 8 | `showHome` 的 `keepScroll` 算了没用 + 自家改动触发全页重渲 |
| 配置/安全/时基 | 12a、14 | token 无开关、各定时器写死、commit 30s 防抖偏密 |
| 新功能/大改 | 5、11、12b、13、15.x | 隐藏 PDF、补文档、联想、筛选、配置/模板/日程/样式 |

## 五、验证与提交流程（每条都走）

1. 改后端解析/接口：先补 `cargo test`；`smoke.ps1` 加断言。
2. 改前端交互：浏览器手点一遍 + `tools/browser-probe.ps1` 补场景；预览勾选类必须跑 `tools/e2e-checkbox.ps1`。
3. 改 `web/src/editor.js`：`cd web; npm run build; cd ..` 再 `cargo build --release`。
4. 提交前：`pwsh -File tools/release-gate.ps1` 全绿；一个改动一个提交，提交信息写**根因** + 编号。
5. 文档：行为变化同步 `docs/noteschain-prd-v05.md`（v0.6 段追加）；问题处理记 `docs/issues-v0.2.md`。

## 六、待你确认的问题（影响我写得对不对）

1. **#1**：我把「杠杠杠」判定为 frontmatter 的 `---`（标签页摘要取首行的 bug），对吗？
2. **#7**：请给一个「代码块前有杠」的最小片段（列表内缩进代码块？还是别的）。
3. **#8**：你说的「进度条回到顶部」是指页面滚动位置，对吗？
4. **#10**：你说的「地服」是指 **diff**，对吗？
5. **#12a**：「cos」是否指 `localhost/本机`？是否接受「token 默认仍开、加 `--no-token` 开关」的安全折中？
6. **#13**：除标签/排序外，还要哪些筛选项（优先级/来源/关键词）？「全部靠右」= 整组控件靠右，对吗？
7. **#15.4**：请给 1~2 张「怪异」截图或指出具体元素，方便我精准改。
8. **#14**：你期望的 Git 提交间隔是多少，还是做成可配置即可？
9. **#15.2**：你要的是「界面内编辑模板」，还是先让内置日记/会议模板可改即可？
10. 优先级是否认可？要不要我按「第一批 P0（1/6/2）」先开工？

> 你确认/修正后，我按批次动手，每批结束跑 release-gate 并回报。

---

## 七、执行进度

### 第一批 P0（代码已完成，待普通终端做浏览器复验）

- ✅ **#1 标签页结果**：后端 `search` 无关键词时改为返回「标签所在行」（新增 `tag_line_snippet`，`first_line` 跳过 frontmatter），前端 `showTag` 显示行号并可跳转。
  - HTTP 已验证：`/api/search?tag=生活` → 第 5 行 `tags: [工作, 生活]`；`/api/search?tag=工作` → 第 12 行 `- [ ] 写周报 #工作`，不再是 `---`。
- ✅ **#6 元数据 tags**：新增 `markdown::frontmatter_tags`（支持 flow/块列表/单值），`collect_tags` 合并 frontmatter 标签。
  - HTTP 已验证：`/api/tags` 出现「工作」「生活」，计数正确。
- ✅ **#2 预览待办可勾选**：`renderList` 渲染真实 `input.preview-todo`；`renderPreviewNodes` 绑定 PATCH 回调；`showFile` 用 `liveContent / editorStale` 与隐藏编辑器解耦（不在 `display:none` 上 setValue，绕开历史死循环坑）；`renderMarkdown/parseList` 的 `data-line` 改用真实文件行号。
- ✅ **#5 隐藏 PDF 导出**（顺带完成）：移除「导出 PDF」按钮，并同步改 `browser-probe` / `release-gate` 断言（改为断言导出按钮不存在）。
- 验证：`cargo test` 32 passed、`cargo clippy` 0 warning、`release-gate -SkipBrowser` 全绿（构建 / 单测 / clippy / 冒烟 / 边界 / 平替）。
- ⚠️ **浏览器层未能在本机跑通**：DSH 沙箱内 Edge 启动即报 `拒绝访问`（mojo/网络通道），非代码问题。需在普通终端跑 `pwsh -File tools/release-gate.ps1`（含 browser 层，已新增 `.preview-todo` 勾选断言 `todoBoxes/todoToggled`）复验 #2。

### 第二批 P1（代码已完成；full release-gate 24 项全绿，含浏览器层）

- ✅ **#4 日记优先级**：`inline()` 增加 `!pN` → `.prio` 徽标（边界与后端 `extract_priority` 一致）。正则已单测。
- ✅ **#9 首页待办行内代码**：`appendTodoText` 同时识别 `` `code` `` 与 `#标签`，代码用等宽样式、标签可点击。
- ✅ **#8 勾选后滚回顶部**：`showHome(preserveScroll)` 在轮询刷新时恢复 `main.scrollTop`；`refreshFromDisk` 传 `true`。
- ✅ **#7 `- ```sql` 代码块**：预览 `renderMarkdown` 围栏识别允许列表项前缀 + 任意缩进；后端 `code_block_flags` 同步（新增 `strip_list_bullet`）避免块内 `- [ ]` 被误判为待办，含单测。
- ✅ **#3 预览双击进编辑**：预览 `dblclick` 进入编辑并置 `autoPreviewOnSave`，Ctrl+S 保存成功后自动回到预览（自动保存不会跳）。
- 验证：`cargo test` 33 passed、clippy 0 warning、`release-gate`（**含浏览器 + Playwright e2e**）**24 项全 PASS**。

### 第三批 P2-配置类（代码已完成；full release-gate 24 项全绿）

- ✅ **#13 首页待办控件靠右**：`buildTodoSection` 把标签筛选 / 排序 / 状态切换包进 `.todo-controls`，整组 `margin-left:auto` 靠右（消除之前每个控件各自 `auto` 造成的居中分布）。
- ✅ **#14 Git 提交时基可配**：默认防抖改为 **300s（5 分钟）**、连续编辑 1800s 强制提交；新增启动参数 `--commit-debounce` / `--commit-force`（秒），并在 `/api/config` 暴露 `commitDebounce/commitForce`。（自动保存 800ms、轮询 5s 保持不变。）
- ✅ **#12a 本地 `--no-token`**：新增启动参数 `--no-token`，默认仍校验 token；开启后不鉴权、启动日志显著告警。HTTP 已验证：无 token 访问 `/` 返回 200、`/api/*` 放行；未开启时无 cookie 仍 401。
- 验证：`cargo test` 33 passed / clippy 0 / release-gate **24 项全 PASS**。

### 待办（后续批次）

- P2 剩余：10（历史 diff 友善化）、15.3（新增日程时间选择）、11（接口文档）、12b（标签联想）、15.1（配置界面：字体/字号/主题）；15.2（模板）、15.4（样式大改）暂缓。
