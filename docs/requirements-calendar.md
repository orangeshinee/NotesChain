# 需求文档：侧栏可折叠月历组件

状态：v1.3 实现完成（布局按用户反馈调整）。Q1–Q5 已由用户确认；实现与端到端验证见第 9 节。

修订历史：v1.1 修正 3 处会导致实现写错的设计问题、4 处事实与行号错误，补齐项目文档同步约定；v1.2 据实测修正 §4.1 / §4.4 / §6 对日期判定与孤儿文件的描述；v1.3 用户看过首页两列方案的实机截图后指出空白太多，改为侧栏可折叠方案（§4.0）。

## 1. 背景与痛点

现在要看之前某一天的日记，必须先点开左侧文件树，逐层展开「年 → 月 → 日」。日记越积越多后，定位一天的代价线性上升。

用户原话：「因为现在如果我想去看之前某一天的日记，不太方便，要点开文件树才行」。

目标：在首页提供一个日历，一天一格，点得到、看得见哪些天写过。

## 2. 现状核查（静态读码 + 运行时实测）

以下结论来自 2026-11 的代码阅读与端到端实测，行号基准固定为 `5b01461`（master HEAD at review time）。实现阶段已用临时笔记目录启动真实服务，POST 验证过接口产物，详见第 9 节。

| 项 | 结论 | 证据 |
|---|---|---|
| 日记路径 | `YYYY/MM/YYYY-MM-DD.md` | `src/api.rs:39-52` `diary_date_from_path` |
| 日记模板 | `docs/templates/daily-note.md`，编译期内嵌 `DAILY_TEMPLATE` | `src/api.rs:21` |
| 模板结构 | frontmatter + `# {{DATE}}` + `## 日程` / `## 待办` / `## 记录` | `docs/templates/daily-note.md` |
| 新建请求体 | `NewNoteRequest` 含 `date: Option<String>` | `src/api.rs:617-631` |
| 日期与路径生成 | `date` 过 `is_iso_date` 形态校验，形态不合法则回退今天；`date` 存在时 `rel` 由后端按 `YYYY/MM/YYYY-MM-DD.md` 生成 | `src/api.rs:695-712` |
| 日记判定（新建） | `is_diary` = `type=="diary"` 或 `date` 存在 或 路径符合日记规范 | `src/api.rs:721-723` |
| 模板选择（新建） | 自定义模板优先，否则日记走 `DAILY_TEMPLATE` | `src/api.rs:731-738` |
| 同名冲突 | `abs.exists()` 返回 `409 conflict` | `src/api.rs:746` |
| Git 收录 | 新建后调用 `st.git.notify()` | `src/api.rs:757` |
| 仅建今天 | `/api/daily` 写死 `Local::now()`，不适用本需求 | `src/api.rs:1089-1091` |
| 已有日记清单 | `/api/files` 按 `file.diary_date` 分年/月，返回 `years[].months[].entries[]` | `src/api.rs:261-300` |
| **FileItem 字段** | 只有 `path/mtime/size/title/todos`，**无 `date` 字段** | `src/api.rs:251-259` |
| 日记判定（索引） | 路径优先，其次首个 `# YYYY-MM-DD` 一级标题 | `src/index.rs:232-236`、`266-280` |
| 日期格式校验 | `is_iso_date` 只校验 `NNNN-NN-NN` 形态，不校验日期真实性 | `src/index.rs:283-292` |
| 前端已有该数据 | `loadSidebar()` 把 `/api/files` 结果存入 `state.files` | `web/app.js:1671-1677` |
| 启动顺序 | `await loadSidebar()` → `route()`，异常被 catch 吞掉后 `route()` 仍执行 | `web/app.js:2653-2658` |
| 跳转日记页 | `go('/file/' + encodeURIComponent(path))` | `web/app.js:2203-2208` |
| 日期路径生成 | `todayPath()` 已有 `YYYY/MM/YYYY-MM-DD.md` 拼接逻辑 | `web/app.js:2195-2201` |
| 确认后新建 | 先例：`window.confirm` → `/api/notes/new` | `web/app.js:1977-1990` |
| API 错误对象 | `api()` 对非 2xx 抛 err，带 `.status` 与 `.code` | `web/app.js:51-58` |
| 首页结构 | `showHome()` 每次 `main.replaceChildren(...)` 全量重建 | `web/app.js:405-446` |
| 侧栏结构 | brand / quicknav / searchbox / panels(文件树 + 标签树) | `web/index.html:11-40` |
| 自定义模板 | 笔记根 `templates/<name>.md`，`/api/notes/new` 的 `template` 参数指定，`/api/templates` 列出 | `src/api.rs:646-668`、`731-738` |

**结论**：本需求主要是前端组件，复用 `/api/files` 与 `/api/notes/new` 两个现有接口，后端零改动。依据是：跳转直接使用 `/api/files` 返回的 `entry.path`，不需要新增字段；新建直接传 `date`，路径由后端生成（见 §4.2、§4.3）。**限定**：若将来要覆盖「非规范路径日记」的日历标记，需给 `FileItem` 加 `date` 字段，届时不再成立（见 §6）。

## 3. 需求描述

1. 新增日历组件，位置为**左侧侧边栏可折叠面板**（用户看过首页两列方案的实机截图后调整，见 §4.0）。
2. 日历中点击某一天：若该日日记已存在，直接打开对应日记编辑页。
3. 若该日日记不存在，弹确认提示，由用户决定是否新增。
4. 用户确认后，按日记模板新建该日日记，并打开编辑页。
5. 用户取消则不做任何写入，停留在当前页面。

## 4. 设计（Q1–Q5 已定）

| 编号 | 决策 | 落地方式 |
|---|---|---|
| Q1 | 左侧侧边栏可折叠面板（v1.3 调整） | 初选「首页右上角常驻卡片」已实现，用户看实机截图后因空白太多否决，改为侧栏 `<details>` 面板；首页恢复单列全宽。详见 §4.0 |
| Q2 | 有日记的日期打点/高亮，今天另做描边强调 | 日期集合命中即加标记 class；今天加独立 outline |
| Q3 | 上一月 / 下一月 + 「回到今天」 | `state` 内维护 `calYear` / `calMonth`，不写入路由 |
| Q4 | 所有日期含未来均可新建 | 不设日期上限，未来日期与过去日期走同一确认流程 |
| Q5 | 内置 `daily-note.md` | `/api/notes/new` 不传 `template`，走 `DAILY_TEMPLATE`（`src/api.rs:738`） |
| 补充 | 周起始日按周一 | 若与预期不符，实现时一行改动 |

### 4.0 卡片布局

**最终方案（用户 2026-11-12 反馈后调整）**：月历作为可折叠面板常驻**左侧侧边栏**，首页恢复单列全宽。用户看过候选 A 的实机截图后指出主内容区右侧空白太多，要求移到左侧栏且可收起。

- 侧栏 `web/index.html` 的 `.panels` 内新增 `<section class="panel panel-calendar">`，用原生 `<details id="calendar-panel" open>` 包裹，与文件树 / 标签的折叠交互一致。日历本体挂在 `#calendar-mount`。
- 折叠状态存 `localStorage('nc.calendarOpen')`，默认展开；刷新后保持。
- 启动链路：`boot()` 开头先 `initSidebarCalendar()`（不依赖接口结果，失败时仍渲染空日历并触发 `calRetryFiles` 兜底），`loadSidebar()` 末尾再 `calRefresh()`，新建日记后立即打点。
- 首页 `showHome()` 只 `wrap.replaceChildren(...parts)`，不再建两列网格。
- 侧栏宽 `var(--sidebar-w)` = 264px，日历不设独立卡片外框，沿用侧栏底色；字号走 `--fs-meta`，日期格 `aspect-ratio: 1/1`。
- 窄屏（`@media max-width: 860px`）侧栏本来就是抽屉，日历随抽屉收起，点汉堡打开；点日期跳转后同步收起抽屉（点击委托加 `.calendar-cell`）。

**已废弃的候选**（保留记录，说明为何不采用）：

- 候选 A：日历卡片插入 `wrap`，`wrap` 顶部改两列网格，卡片占右列。已实现并验证过，但实机截图显示日历卡片下方留出大片空白、主内容被挤窄，用户否决。
- 候选 B：日历卡片绝对定位到 `docHead` 右侧空白区。会与 `.doc-head > .status` 的 `margin-left: auto`（`web/style.css:493-494`）冲突。

注：主内容区右侧仍有留白，来自 `.wrap { max-width: 860px }`（`web/style.css:520`）这一既有版式约束，设置里的「宽屏」开关（`body.wide`）可去掉。与日历改动无关，本次不改全局版式。

### 4.1 数据来源与翻月状态

**日期集合**：`state.files` 由 `loadSidebar()` 拉取（启动顺序已验证：`web/app.js:2653` 先于 `route()` 的 `2658`）。从 `groups[kind=diary]` 取 `year` / `month`，再从 `entry.path` 末段解析出 `YYYY-MM-DD`，组成 `Map<YYYY-MM-DD, path>`，同时供打点标记与跳转使用。

`FileItem` 无 `date` 字段（`src/api.rs:251-259`），所以「日」只能从 `entry.path` 末段取。判据是**末段文件名是否为合法日期**，不是路径是否恰好三段：实测四段路径 `MyNotes/notes/2026/11/2026-11-09.md` 仍会被标记。只有末段无日期、靠首个 `# YYYY-MM-DD` 一级标题被判为日记的文件（`src/index.rs:266-280`）虽有 year/month 分组却取不到「日」，日历不标记。这是明确的范围取舍，见 §6。

**异常兜底**：`loadSidebar()` 抛错时会被外层 catch 吞掉，`route()` 仍执行，此时 `state.files` 为 `null`（`web/app.js:2653-2658`）。日历组件必须处理该情况：自行 `GET /api/files`，或渲染为空日历并给出提示，不得抛异常中断首页。

**翻月状态必须放在 `state`**：`showHome()` 每次 `main.replaceChildren(...)` 全量重建首页（`web/app.js:411`、`435-436`），且 375 / 392 / 466 / 520 / 737 等多处会重新调用。若 `viewYear` / `viewMonth` 只存在组件闭包或 DOM 上，用户翻到三月后勾一条待办就会弹回当月。在 `state` 中加 `calYear` / `calMonth`（以及「用户是否手动翻过月」标记），组件渲染时从 `state` 读、翻月时写回 `state`。「回到今天」重置为当前年月。

### 4.2 跳转

- **点击已有日记**：直接用 `/api/files` 返回的 `entry.path` 跳转，`go('/file/' + encodeURIComponent(entry.path))`。**不要用日期重拼路径**：diary 组里可能存在非规范路径的文件（见 §4.1），重拼会跳到不存在的文件，再触发「是否新建」，导致生成重复日记。
- **新建成功后**：用 `/api/notes/new` 响应里的 `path` 字段跳转，同样不重拼。
- 现有的 `todayPath()`（`web/app.js:2195`）服务于侧栏「今天」按钮，本需求不改它，也不需要抽出带参版本。

### 4.3 新建

点击不存在的日期 → `window.confirm`（与现有交互一致）→ `POST /api/notes/new`，body 只传 `{ "date": "YYYY-MM-DD" }`，**不传 `path`**。

理由：`is_diary_request` 在 `date` 存在时为真，后端会按 `YYYY/MM/YYYY-MM-DD.md` 生成 `rel`（`src/api.rs:702`、`707`），路径规则只有一处，避免前后端各拼一套产生漂移。

传规范 `path` 也可行（`is_diary` 判定包含 `diary_date_from_path(&rel)`，`src/api.rs:721-723`），但那样前端得自己拼路径，与「复用后端路径生成」的目标冲突，因此不采用。

新建成功后调用 `loadSidebar()` 刷新 `state.files`，否则日历不会立即标记该日，且文件树不更新。注意 `loadSidebar()` 会重绘整个文件树，展开状态会重置，这是既有行为，可接受。

### 4.4 边界

- 路由切换前 `route()` 会先 flush 未保存改动（`web/app.js:1889-1898`）。日历跳转走 `go()` 即复用该保护，不需额外处理。
- 新建返回 `409 conflict`（`src/api.rs:746`）时，视为「已被其他端创建」，直接跳转而不报错。`api()` 抛出的 err 带 `.status` 与 `.code`（`web/app.js:51-58`），按 `err.status === 409 && err.code === 'conflict'` 精确判定；其它错误照常提示，不要一律吞掉。
- 后端模板渲染用 `chrono::Local::now()` 生成 `created` 时间戳（`src/api.rs:690-691`），补建过去日期时 `created` 是当下时间，`title` / `# 日期` 是目标日期。此差异需在文档中说明，避免误读。
- **Q4 允许未来日期的后果**：补建过去日期与预建未来日期在 frontmatter 上无法区分，两者 `created` 都是当下时间。不能靠 `created` 反推实际写作时间。当前需求不要求区分，记录在此以免后续基于 `created` 做统计时踩坑。
- 未来日期的日记会被 `/api/files` 正常收录并显示在文件树与日历中，这是 Q4 选定后的预期行为。
- **日期校验有两类不同的失效，需分别防**：
  - 形态不合法（如 `2026-2-03`）→ `is_iso_date` 拒绝 → `filter` 丢弃 → 静默回退 `today`（`src/api.rs:695-701`），**会在今天误建文件**。
  - 形态合法但日期不真实（如 `2026-02-31`）→ `is_iso_date` 只校验 `NNNN-NN-NN` 形态（`src/index.rs:283-292`），会通过 → 按 `2026/02/2026-02-31.md` 建成文件。**实测（2026-11-12）该文件仍会进 diary 组**：`create_note` 的 `is_diary` 判定含 `body.date.is_some()`，索引按写入内容里的 `# 2026-02-31` 标题兜底判定通过。所以它是「进了分组但日期非法」的孤儿，不是「不进分组」。前端 `calValidDate` 同时校验形态与真实日期，会跳过它，日历不标记。
  - 日历组件生成的日期来自 JS `Date`，两类都不会自然出现。但组件仍须校验前端生成的字符串后发送，且验收需覆盖两类（第 11 条）。

## 5. 决策记录

Q1–Q5 全部已确认，无遗留待决事项。

| 编号 | 用户选定 | 备选（未采纳） |
|---|---|---|
| Q1 | 左侧侧边栏可折叠面板（v1.3，用户看截图后改选） | 首页右上角常驻卡片（初选，已实现后否决）/ 按钮点开浮层 / 两处都放 |
| Q2 | 有日记打点高亮，今天描边强调 | 只高亮今天 / 加待办数 / 不标记 |
| Q3 | 上下月 + 回到今天 | 加年月下拉 / 仅当月 |
| Q4 | 所有日期含未来 | 今天及过去 / 仅过去 |
| Q5 | 内置 `daily-note.md` | 自定义模板优先 / 确认时选 |

## 6. 非目标

- 不新增后端接口，不改 `FileItem` 结构。
- 不修改日记模板结构。
- 不在日历上聚合展示日程、待办内容，仅展示存在性。
- **不覆盖文件名末段无日期的日记**：末段无日期、仅靠首个 `# YYYY-MM-DD` 标题被判为日记的文件（如 `raw/标题判定.md`），虽有 year/month 分组却取不到「日」，日历不标记。若要覆盖，需后端给 `FileItem` 加 `date` 字段，属于另一个需求。末段是合法日期但路径层级不规范的（如四段路径），日历正常标记并用 `entry.path` 跳转。
- 不改变现有文件树、路由、保存机制。

## 7. 验收标准

1. 首页右上角常驻月历卡片，显示当月日期网格，跨月切换（上一月 / 下一月 / 回到今天）正常。
2. 有日记的日期显示打点或高亮；今天有独立于高亮的描边强调。
3. 点击有日记的日期，打开 `entry.path` 对应的编辑页，内容与该文件一致。**不使用日期重拼的路径**。
4. 翻月后触发任一会重建首页的操作（如勾选待办），日历停留在用户所翻的月份，不弹回当月。
5. 点击无日记的日期（含未来日期），弹出确认；取消后不产生任何文件、不跳转。
6. 确认后按内置 `daily-note.md` 创建：frontmatter 的 `title` 与正文 `# 日期` 均为目标日期，且含 `## 日程` / `## 待办` / `## 记录` 三段。
7. 新建成功后用响应返回的 `path` 打开该日编辑页，且文件树与日历标记同步更新，无需手动刷新。
8. 有未保存改动时从日历跳转，不丢内容（复用 `route()` 的 flush 保护，`web/app.js:1889-1898`）。
9. 补建的日记被 Git 自动提交链收录（`st.git.notify()` 已由 `/api/notes/new` 调用，`src/api.rs:757`）。
10. 新建返回 `409 conflict` 时直接跳转，不弹错误；其它错误（如 400、500）照常提示。
11. 分别构造两类异常日期并确认前端拦下：① 形态不合法（如 `2026-2-03`），不得误建今天的日记；② 形态合法但日期不真实（如 `2026-02-31`），不得建成孤儿文件。
12. `state.files` 为 `null`（模拟 `/api/files` 失败）时，首页正常渲染，日历不抛异常。
13. 窄屏下日历卡片不遮挡日程与待办区，sidebar 折叠行为不受影响。
14. 项目文档同步：按 `AGENTS.md:24` 与 `docs/agents/workflow.md:54` 的约定，在 `docs/noteschain-prd-v05.md` 的 v0.6 段追加本次产品行为变化的修订说明，不直接改历史章节。

## 8. 风险与待查

- **已选定**：布局采用 §4.0 最终方案 —— 月历作为可折叠 `<details>` 面板常驻左侧栏，首页单列全宽。候选 A（首页两列）已被用户查看实机截图后否决。
- **已验证（2026-11-12，本机真实浏览器）**：验收第 13 条窄屏布局。用本机 Chrome 145 + playwright 在 1440×900 / 820×1180 / 375×812 三个视口实测，量了 bounding box 并截图人工复核。窄屏侧栏为抽屉，月历随之收起、点汉堡打开，主内容铺满 375px / 820px，无横向溢出。
- **已知取舍**：主内容区右侧仍有留白，来自 `.wrap { max-width: 860px }`（`web/style.css:520`）既有版式约束，设置里的「宽屏」开关可去掉。与本次改动无关，未改全局版式。
- **已知取舍**：侧栏 264px 宽，日期格约 33px，触控热区偏小；折叠箭头与其他面板一致，视觉上偏淡。实机截图复核过，未发现截断或溢出。
- **未确认**：周起始日按周一为默认值，若与预期不符实现时一行改动。

## 9. 实现与验证记录（2026-11-12）

**改动（v1.3 为当前状态）**

- `web/index.html`：`.panels` 内新增 `<section class="panel panel-calendar">` + `<details id="calendar-panel">` + `#calendar-mount`。
- `web/app.js`：新增 `state.calYear` / `calMonth` / `calPinned`；新增日历组件（日期集合、月历渲染、翻月、跳转、新建、异常兜底）；新增 `initSidebarCalendar()`（折叠状态持久化）；`loadSidebar()` 末尾 `calRefresh()`；`boot()` 开头独立初始化；窄屏点击委托加 `.calendar-cell`。`showHome()` **恢复单列**（v1.2 的两列网格已删除）。
- `web/style.css`：新增 `.panel-calendar` 折叠标题与紧凑日历样式，颜色只走 `:root` 变量；删除 `.home-grid` 与窄屏两列覆盖规则。
- `docs/noteschain-prd-v05.md`：v0.6 段第 10 条改为侧栏方案；`docs/issues-v0.2.md` #50 同步。
- 后端零改动。

**验证（均在临时笔记目录 `~/.hermes/cache/scratch/nc-e2e` 上跑真实服务，未碰用户笔记库）**

| 项 | 方法 | 结果 |
|---|---|---|
| 构建 | `cargo build --release` | 通过，17.06s |
| JS 语法 / CSS 括号 | `node --check web/app.js`、括号计数 | 通过 |
| 接口产物 | POST `date=2026-10-05`（过去）、`date=2027-03-15`（未来） | title 与 `# 日期` 均为目标日期，`created` 为当下时间，三段模板齐全 |
| 409 冲突 | 重复 POST 同一日期 | HTTP 409 + `code: "conflict"`，前端按 `.status` / `.code` 精确判定 |
| 两类日期失效 | POST `2026-2-03` / `2026-02-31` | 前者静默回退今天、后者建成并进入 diary 组；前端 `calValidDate` 两类都拦下 |
| 纯函数 | 抽 `calValidDate` / `calDiaryMap` / `calKey` / `calEnsureView` 跑 6 例日期 + 真实 `/api/files` 数据 | 6/6 + 6/6 + 补零 + 翻月状态保持全通过 |
| DOM 渲染与交互 | jsdom 加载真实首页，14 项断言 | 14/14：卡片、周一、今天描边、打点、翻月、重建保持、回今天、`entry.path` 跳转、取消不写入、无运行期错误 |
| 确认新建链路 | jsdom + 真实 POST 转发 | 只传 `date`，跳转使用响应 `path` |
| 真实浏览器布局（v1.2 两列方案） | 本机 Chrome 145 + playwright，3 视口 | 29 项中 28 通过；唯一失败是浏览器自动请求 `/favicon.ico` 的 404，服务端本就无该路由，与代码无关，排除后 28/28 |
| 真实浏览器布局（**v1.3 侧栏方案，当前**） | Chrome 145 + playwright，1440×900 / 820×1180 / 375×812，量 bounding box + 截图人工复核 | **33/33**：日历挂在侧栏 `#calendar-mount`、首页单列全宽（日程区 780px）、与日程不重叠、无横向溢出；折叠 / 展开 + `localStorage` 持久化 + 刷新后保持；翻月 / 回今天 / `entry.path` 跳转 / 回首页日历仍在；窄屏抽屉随之收起、点汉堡打开；主内容铺满 375 / 820px |
| 侧栏方案截图复核 | 桌面、折叠态、375px 抽屉打开三张截图人工看 | 日历在侧栏、折叠箭头可见、收起后侧栏紧凑文件树上移、无异常留白；窄屏抽屉内日历完整无裁切 |
| 提交前闸门 | `bash tools/release-gate.sh`（同批次补齐的 Linux 版闸门） | 11/11 全绿，exit 0：前端静态检查、build `--locked`、`cargo test` 66 passed、clippy `-D warnings` 零警告、二进制可用性、临时目录服务接口冒烟 |
| 闸门负向测试 | 副本里注入 CSS 语法错误 | 闸门变红、exit 1、7 项 FAIL，确认不是「假绿」 |

注：v1.3 测试里「收起后日历不可见」一度报错，查明是 Chrome 145 对 `<details>` 收起内容保留布局尺寸，`getBoundingClientRect()` 仍有高度，须用 `checkVisibility()` 判可见性。产品代码无需改动，测试断言已修正。

**验收标准对照**：第 1–14 条全部已验证。

**提交**：功能与 Linux 闸门分两个提交（一个改动一个提交，见 `docs/agents/workflow.md`）；只做本地提交，未 push、未开 PR。
