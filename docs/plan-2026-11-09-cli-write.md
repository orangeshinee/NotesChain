# 定向写入 CLI 方案（待确认）

日期：2026-11-09 ｜ 适用版本：master `c4e4284` ｜ 状态：已完成（实现 + review 修复）；单元与端到端：已验证；`release-gate.ps1`：未验证（本机无 pwsh）；review：已完成

核查范围：`src/*.rs`、`docs/api.md`、`docs/agents/*.md`、`Cargo.toml`、`README.md`、`tools/`。事实结论均标 `路径:行号`；设计草案与待确认项单独标注，不与现状混写。

> 文中 `路径:行号` 是 v2 撰写时基于 `c4e4284`（实现前）的快照。实现与 review 修复后行号会漂移，排查请以函数名 / 文件路径为准；§2、§5 描述的是改动前的现状。

v2 依据本轮 review 修订：拆开「底层能力」与「调用方责任」，补 CLI 定位与鉴权（原 §8 的 `session.json` 方案有循环依赖且会把 token 提交进 Git），补命令契约、防呆与退出码，修正 BOM 后果、标题别名、pitfalls 条号、日程「疑似」表述、验证范围。

## 1. 结论

**能支持，但缺口是三层，不是两层，而且第三层不只是「补参数」。**

| 层 | 现状 | 缺口 |
| --- | --- | --- |
| 写入底层（`src/api.rs:770-820`） | `insert_into_section(content, heading, line)` 只负责「指定标题 + 追加一行」，段不存在自动建段，返回插入行号 | 无。**「指定文件」不在这层**，是调用方 `st.resolve(&rel)` 的责任（`src/api.rs:841, 899`） |
| HTTP 接口（`src/api.rs:55-88`） | `POST /api/todos/append` 有 `path`（`src/api.rs:753-759`），标题写死；`POST /api/schedule/append` 只有 `text`（`src/api.rs:875-878`），文件与标题都写死 | 日程接口补 `path`；两个接口补 `section`；日程的缺省标题要保留别名兼容 |
| CLI（`src/main.rs:19-49`） | 只有 7 个 server 启动参数，`clap::Parser`，无 `Subcommand`，`axum::serve` 常驻（`src/main.rs:87`） | 新增子命令，同时必须解决「CLI 怎么找到 server」「怎么带 token」「怎么防呆」三件新增工作 |

把 CLI 当成一个新写入入口，就会带出三件原接口不用管的事：**定位**（root / port）、**鉴权**（token 落用户级登记表，绝不落笔记目录）、**安全边界**（重复执行、文件新建、非交互场景）。v1 把这部分压成「token 传给 CLI」一句，是它不能作为开工依据的主因。

## 2. 现状核查

### CLI 层

`Args` 的 7 个参数：`--dir / --port / --links / --no-git / --no-token / --commit-debounce / --commit-force`（`src/main.rs:21-49`）。其中 **`dir: PathBuf` 是必填**（`src/main.rs:23-24`，无 `default_value`、非 `Option`）。`main()` 顺序是 `AppState::new` → `rebuild_index` → `watch::start` → `axum::serve`（`src/main.rs:53-87`），没有「执行一条命令就退出」的路径。

这一点决定了 §7.2 的兼容性改造：加子命令必须把 `dir` 改成 `Option<PathBuf>`，否则任何不带 `--dir` 的子命令都会被 clap 拦下。旧的 `noteschain --dir X --port 8090` **行为**不变，但 `Args` 结构和错误分支要动。

### 接口层

| 接口 | 目标文件 | 目标标题 | 依据 |
| --- | --- | --- | --- |
| `POST /api/todos/append` | `body.path`，缺省今天日记 | 写死 `## 待办` | `src/api.rs:836-839, 853`；`docs/api.md:199-209` |
| `POST /api/schedule/append` | 写死今天日记 | `insert_into_section_any(&["## 日程", "## 今日日程"])`：取第一个存在的，全不存在则用 `headings[0]` 即 `## 日程` 新建 | `src/api.rs:897-898, 911`；`src/api.rs:761-768`；`docs/api.md:306-310` |

前端两个快捷添加入口只传 `text`（`web/app.js:492, 695`）。扩展字段保持缺省语义不变即可零改动。

### 可复用点

- `insert_into_section`（`src/api.rs:770-820`）：精确匹配标题行；段尾为下一个一级/二级标题前，去掉段尾空行后插入；标题不存在时在文件末尾追加该标题与内容。
- `insert_into_section_any`（`src/api.rs:761-768`）：候选标题列表，兼容老笔记的标题别名。
- `AppState::resolve()`（`src/state.rs:111-152`）：路径规范化 + 根目录包含校验。
- `todo_write::atomic_write`（`src/todo_write.rs:112-124`）：同目录临时文件 + 改名。

## 3. 语义约束

### 3.1 待办写任意标题都有效，日程不行

- 待办：`markdown::parse_todos` 全文逐行扫，只认 `- [ ]` 前缀并排除代码块（`src/markdown.rs:172-186`）。写进 `## 会议纪要` 下的待办照样进首页汇总。
- 日程：`schedule::parse` 只认二级标题 `日程` / `今日日程` 段内的行（`src/schedule.rs:34-45, 146-155`）；`GET /api/schedule` 用 `f.diary_date == Some(date)` 找文件（`src/api.rs:1415`）。写进自定义标题或非日记路径的日程行不会出现在日程视图。
- 「疑似日程」兜底很窄：`suspected_outside_section`（`src/schedule.rs:141-165`）只收行首能解析出时间的行，且 `docs/api.md:304` 写明 `suspected` 仅 `range=today` 返回。**没有时间的日程文本是静默不可见**，不会有任何提示。
- 结论：**「写到哪个标题下」对日程不是自由参数**。要么限定白名单，要么同步改解析器与产品行为。

### 3.2 目标文件不存在时

两个 append 都会用日记模板新建（`src/api.rs:847-851, 904-909`；模板 `docs/templates/daily-note.md` 含 frontmatter + `## 日程/待办/记录`），且新建前要 `create_dir_all` 建父目录（`src/api.rs:854-858, 912-916`）。指定 `--file raw/周报.md` 而文件不存在时，套日记模板会把日记骨架塞进普通笔记。CLI 默认应拒绝新建，把选择交给 `--create`，见 §7.3。

### 3.3 鉴权

`auth` 从 `Cookie: nc_token=` 取值（`src/api.rs:157-179`），不接受 header 或 query。token 每次进程随机生成（`src/state.rs:173-177`），只存在进程内存、只在启动日志打印一次（`src/state.rs:21`、`docs/agents/workflow.md:19`），另由 `src/cli.rs` 存一份到用户级登记表供 CLI 读取，绝不写进笔记目录。`--no-token` 时 token 为空串，`auth` 直接放行（`src/state.rs:73`、`src/api.rs:166-169`）。`Cargo.toml:8-19` 当前无 HTTP client 依赖。

**token 不能写进笔记目录。** `src/git.rs:357-377` 自动生成的默认 `.gitignore` 只有 `*.tmp`、`.noteschain.*.tmp`、`.DS_Store`、`Thumbs.db`、`desktop.ini`，没有 `.noteschain/` 目录规则；`git.rs:73` 启动时会调 `ensure_gitignore`，之后自动提交队列会收走目录里的新文件。`src/index.rs:303-340` 的忽略实现也只支持 `name` / `dir/` / `*.ext` 三种写法。所以「在笔记根写 session.json」这个方案会把 token 提交进版本历史，v1 的 §8 第 4 条作废。

### 3.4 标题别名

`docs/issues-v0.2.md:83` 记录模板标题从「今日日程」改成「日程」，老笔记仍用 `## 今日日程`。`insert_into_section` 是精确匹配（`src/api.rs:775`），CLI 若把 `--section` 默认成 `## 日程`，对老笔记会找不到段而在文件末尾**新建第二个日程段**。缺省必须沿用 `insert_into_section_any` 的候选列表，或让 `--section` 缺省走别名解析。

## 4. 写入链路

按 `docs/agents/pitfalls.md` 第 6、7 条（第 9 条是 browser-probe 验证脚本腐坏，属于 §9 验证范围，不在这里）：

1. 路径一律过 `AppState::resolve()`，禁止自己 `root.join()`。
2. 按文件加锁 `lock_for`（`src/state.rs:102-108`；用法 `src/api.rs:843-844`）。
3. 目标文件不存在且允许新建时，先 `create_dir_all` 建父目录（`src/api.rs:854-858, 912-916`）。
4. `todo_write::atomic_write` 落盘。
5. 写后 `index.upsert` + `git.notify()` + `revision.fetch_add`（`src/api.rs:859-867, 917-925`）。

红线（`AGENTS.md:10`、`docs/agents/data-conventions.md:3-5`）：绝不改写已有笔记，只允许「追加一行」或「新建文件写模板」。**CLI 是新的写入入口，这条红线同样适用，且要落到防呆上（§7.3）。** `text` 禁止换行（`src/api.rs:830-832, 888-890`）。

## 5. 扩展前建议先修的现有坑

1. **CRLF 被整篇改写**：`insert_into_section` 用 `lines()` + `join("\n")` 无条件输出 LF（`src/api.rs:800-805`）。Rust `str::lines()` 会把 `\r\n` 的 `\r` 一起剥掉，本轮用最小 `rustc` 探针确认过：CRLF 输入经 `lines()` + `join("\n")` 后不含 `\r`。这与 `docs/agents/data-conventions.md:56`「按原 eol 还原」冲突。索引侧已有 `canonical_text` 规范化口径（`src/index.rs:201-206`），写入侧没还原。
2. **BOM 口径不一致**：append 用 `std::fs::read_to_string` 直读（`src/api.rs:848, 906`），不剥 BOM；索引用 `canonical_text` 剥（`src/index.rs:183, 203-206`）。本轮 `rustc` 探针确认：BOM 只附在第一行，不影响 `## 待办` 这类标题行的精确匹配，但 `lines()` + `join("\n")` 会把 BOM **原样写回**。真正后果是磁盘原文与索引规范化文本口径不一致，影响版本号对齐（`src/index.rs:182, 209-212`），不是「首行判定失配」。
3. **代码块误判有两处，不是一处**：找标题时只比 `text.trim() == heading`（`src/api.rs:773-779`），找段尾时只看 `starts_with('#')` 且 level <= 2（`src/api.rs:785-793`），两处都没用 `markdown::code_block_flags`。代码块里出现一行 `## 待办` 既会误命中标题，也会误判段尾。修的时候要一起扫。

这三条现有 append 已经带着，参数开放后影响面放大。建议先补单测再动接口。

## 6. 方案对比（待确认）

| 方案 | 做法 | 代价 |
| --- | --- | --- |
| A 直写 | CLI 子命令直接读写文件，不启 server | `lock_for` 是进程内 Mutex（`src/state.rs:102-108`），server 同时在跑时跨进程并发写会互相覆盖；`git.notify()` / `index.upsert` / `revision` 都在 server 内存里，CLI 直写不进 Git 提交队列。`watch.rs:79-111` 只更新索引与 revision，**不**触发 `git.notify()`。要补齐得让 CLI 自己调 `git.rs` 或做跨进程协调 |
| B（推荐） | CLI 子命令作为 HTTP 客户端，调 server 上扩展后的接口 | 锁、索引、Git 队列、前端 revision 全走现成链路。代价：**server 必须常驻**；CLI 要先解决定位与鉴权（§7.1）；需加 HTTP client 依赖或手写最小 HTTP POST（仅连 127.0.0.1，无 TLS） |
| C 只改 API | 扩展两个 HTTP 接口，不动二进制，外部用 curl / 脚本 / Hermes 调 | 改动最小，但「CLI」没落地，定位与 token 问题仍在 |

推荐 B 的理由：写入后要同时满足界面即时刷新、索引更新、Git 版本链三件事，这三条目前只在 server 进程内闭合。A 要重做这套一致性，成本高于解决定位与鉴权。**B 的成立前提是 §7.1 先落地**，否则命令连 server 都找不到。

## 7. CLI 契约设计（B 方案下，均为草案，未实施）

### 7.1 定位与鉴权：先破循环依赖

CLI 需要知道 `root`、`port`、`token`。把 runtime 文件放在 `<root>/.noteschain/` 会造成循环（要先知道 root 才能读到 root），且有 §3.3 的 Git 泄漏风险。

| 候选 | 做法 | 代价 |
| --- | --- | --- |
| a（推荐）用户级实例登记表 | server 启动时在**用户级固定位置**登记 `{root, port, token, pid, started_at}`，退出时注销。Linux/macOS 用 `$XDG_RUNTIME_DIR/noteschain/instances.json`（回退 `~/.cache/noteschain/`），Windows 用 `%LOCALAPPDATA%\NotesChain\instances.json`；文件权限 600 | 要处理陈旧条目（server 崩溃残留）、多实例选择、Windows 权限语义；token 落用户目录而非笔记目录，不进 Git 历史 |
| b 显式参数 | CLI 带 `--dir` / `--port` / `--token`，无隐藏状态 | 每次都要从启动日志抄 token，token 重启即失效，实际不可用；适合调试或一次性脚本 |
| c 免鉴权本机通道 | server 另绑一个仅 `127.0.0.1`、不走 `auth` 的端口给 CLI | 多一个监听面，等于给本机任意进程开了写入口，安全边界比现在宽 |

推荐 a，配套约束：

- **token 绝不写入笔记目录**，这是硬约束，不是「加条 .gitignore」就能了事（§3.3）。
- 多实例：登记表是列表，按 `root + port` 区分。CLI 未指定 `--dir` 时，只有一个活实例就自动选；多个则报错并列出实例，要求 `--dir` 或 `--port` 消歧。
- 陈旧条目：CLI 连接失败时探测端口，标记该条目失效并给出明确提示，**不自动清理、不自动拉起 server**。
- `--no-token` 实例：登记 `token: ""`，CLI 直连无需鉴权，这是最简路径。
- 登记表与 `AppState::new` 的 `no_token`（`src/state.rs:73`）、启动打印的 token 地址（`src/main.rs:79-85`）必须同源，避免两份状态不一致。

### 7.2 命令形态与兼容性

`Args` 改造（`src/main.rs:19-49`）：

- `dir: PathBuf` → `dir: Option<PathBuf>`；加 `#[command(subcommand)] command: Option<Commands>`。
- `command == None` 时走原 serve 流程，并校验 `dir` 存在，缺失时给与现在一致的明确错误。**旧命令的行为不变，但 `Args` 结构和这条错误分支确实要改**，不能说「一字不改」。
- `command == Some(...)` 时不构建索引、不起 watch、不 `axum::serve`，执行完即退出。

命令草案：

```
noteschain add todo     --file 2026/11/2026-11-09.md --section "## 待办" --text "买牛奶 #生活"
noteschain add schedule --file 2026/11/2026-11-09.md --section "## 日程" --text "15:00-16:00 周会 @会议室"
noteschain add line     --file raw/周报.md --section "## 记录" --text "和财务确认口径"   # 见 §8 第 1 条
noteschain add todo     --text "发周报" --dry-run --json
```

`add line` 是通用追加，用来覆盖你原话里「例如日程 / 待办」之外的类型。**是否纳入本次范围待确认（§8 第 1 条）**，文档先把它作为可选第三个子命令列出，不擅自定。

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `--file` | 今天 `YYYY/MM/YYYY-MM-DD.md` | 相对 notes 根，过 `resolve()`；不存在时默认拒绝，除非 `--create` |
| `--section` | todo = `## 待办`；schedule = 候选列表 `["## 日程", "## 今日日程"]`；line = 必填 | 传参格式见 §8 第 6 条。**日程缺省走别名列表，不用单个 `## 日程`**（§3.4） |
| `--text` | 必填 | 单行；todo 自动补 `- [ ] `，schedule 自动补 `- `，line 原样 |
| `--dir` / `--port` | 从实例登记表推断 | 显式覆盖，用于多实例消歧或调试 |
| `--create` | 关 | 允许目标文件不存在时新建；新建策略见 §8 第 5 条 |
| `--unique` | 关 | 目标段内已有完全相同行时跳过，返回该行号，不重复追加 |
| `--dry-run` | 关 | 只输出将要插入的行与目标位置，零写入 |
| `--json` | 关 | 结构化输出，见 §7.4 |

### 7.3 防呆与安全边界

CLI 会改用户笔记文件，按 mutating 命令处理：

- **默认拒绝新建**。目标文件不存在直接报错，不用日记模板偷偷建文件，也不建父目录。`--create` 才允许，且新建时按 `--file` 是否日记路径选模板（§8 第 5 条）。
- **默认不自动重试**。HTTP 超时或 5xx 时报错退出，由调用方决定是否重跑。自动重试会把「请求到了但响应丢了」变成重复落盘。需要重跑时用 `--unique` 兜底。
- **重复执行**：默认追加语义（两次写两行）；`--unique` 提供幂等选项。是否默认开启 `--unique` 待确认（§8 第 7 条）。
- **`--dry-run` 零写入**，覆盖 `--create` 场景，供 Hermes / cron 等非交互调用方先确认目标位置。
- **非交互路径**：`--json` 走 stdout、错误走 stderr，不依赖 TTY，不弹确认。
- **红线绑定**：命令只做「追加一行」或「新建文件写模板」，不做整篇格式化、不补 frontmatter、不改 eol（`AGENTS.md:10`、`docs/agents/data-conventions.md:3-5`）。§5 的 eol 坑修好前，CLI 不得上线。

### 7.4 输出、退出码与接口扩展

`--json` 输出（成功与 dry-run 同构，便于脚本解析）：

```json
{ "ok": true, "action": "inserted", "path": "2026/11/2026-11-09.md",
  "section": "## 待办", "line": 18, "version": "...", "text": "- [ ] 买牛奶" }
```

`action`：`inserted` | `skipped`（`--unique` 命中） | `dry_run`。失败时 `ok: false`，附 `code` 与 `message`。

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功，含 `skipped` / `dry_run` |
| 2 | 参数错误（含 `--section` 格式非法、缺 `--text`、路径越界） |
| 3 | 目标文件不存在且未带 `--create` |
| 4 | 定位失败（无实例 / 多实例未消歧 / server 未启动 / 鉴权失败） |
| 5 | 写入冲突 / 服务端错误（`internal`、`conflict`） |

`--json` 模式下成功结果走 stdout，错误 JSON 走 stderr，stdout 留给脚本解析成功结果。

`--help` / `--version` 由 clap 提供；退出码表、stdout/stderr 分流、非 TTY 行为要实测，见 §9。

HTTP 侧扩展现有两个接口，并新增 `POST /api/lines/append`（`add line` 已按 §8 第 1 条纳入）：

```
POST /api/todos/append     { "text": "...", "path": "...", "section": "## 待办" }
POST /api/schedule/append  { "text": "...", "path": "...", "section": "## 日程" }
```

- `section` 缺省沿用现值，前端 `web/app.js:492, 695` 零改动。
- 日程的 `section` 缺省必须是候选列表，不是单个字符串（§3.4）。
- 错误码沿用 `ApiError` 固定枚举，不随意新增 `code` 字符串（`docs/agents/workflow.md:33`）。
- `--unique` 若实现，去重判断放在 server 侧目标段内，不改写入语义。

## 8. 待确认（已按推荐确定）

> 2026-11-09 按「按你推荐」授权，八条已定：1 三种类型（含 `add line`）；2 B 走 HTTP；3 a 用户级实例登记表；4 server 未启动明确报错；5 默认拒绝 + `--create`（日记路径用日记模板，其余用普通模板）；6 完整标题行 + 纯标题名自动补 `## `，日程限 `## 日程` / `## 今日日程` 白名单；7 `--unique` 默认关；8 不改日程视图。以下保留原选项作为决策记录。

1. **写入类型边界**：只做 `add todo` / `add schedule` / 再加通用 `add line`（推荐，对应你原话的「例如」）/ 其他。这决定接口要不要新增第三个入口。
2. **CLI 落地形态**：A 直写 / B 走 HTTP（推荐）/ C 只改 API。
3. **定位与鉴权**：a 用户级实例登记表（推荐）/ b 显式参数 / c 免鉴权本机通道。
4. **server 未启动时**：明确报错并提示启动命令（推荐）/ CLI 自动拉起 server。自动拉起有副作用，需单独授权。
5. **目标文件不存在时**：默认拒绝 + `--create`（推荐）/ 沿用现有日记模板 / 按普通笔记模板新建。`--create` 时日记路径与普通路径分别用哪个模板也要定。
6. **`--section` 传参格式**：带 `##` 的完整标题行 / 纯标题名；以及日程是否允许任意标题（允许则同步改 `src/schedule.rs` 解析器并更新 PRD，不允许则限定 `日程` / `今日日程` 白名单）。
7. **`--unique` 是否默认开启**：默认关（保持纯追加语义）/ 默认开（重复执行更安全，但会改变「写两行」的预期）。
8. **是否把任意标题下的日程纳入日程视图**：这会改产品行为，需同步 `docs/noteschain-prd-v05.md` v0.6 段。

## 9. 分阶段与验证

| 阶段 | 内容 | 验证 |
| --- | --- | --- |
| 1 | 抽 `insert_into_section` 到公共写入模块，修 §5 三个坑（两处代码块判定一起扫） | 新增单测：CRLF 保持、BOM 保留与版本号、代码块内标题、代码块内段尾、段不存在自动建段、`###` 不截断段尾、标题别名 |
| 2 | 两个 append 接口加 `path` / `section`，补错误码与 `docs/api.md` | 旧调用零改动回归；接口冒烟；日程缺省走候选列表 |
| 3 | 实例登记表 + `clap` 改造 + `add todo` / `add schedule`（`add line` 视 §8 第 1 条） | CLI 契约矩阵（下方）；写入后检查落盘行、索引汇总、Git 队列、界面 revision |
| 4 | 更新 `README.md`、`docs/agents/workflow.md`、`docs/issues-v0.2.md`、`docs/api.md` | 双平台构建与静态检查；真实笔记目录平替 |

验证分四块，不只看库测试：

- **单元**：上表阶段 1 的各项。
- **构建与静态**：`cargo build --release`、`cargo test`、`cargo clippy`。`README.md:74` 写明 CI 在 Windows 与 Ubuntu 双平台跑这三项，Linux 侧可直接跑；Windows 侧另跑 `pwsh -File tools/release-gate.ps1`（`AGENTS.md:9`）。两边结果都要报，不写成「只在 Windows 侧跑」。
- **CLI 契约**（对应 `check` 的 CLI Command Surface）：`--help` / `--version`；退出码矩阵 0/2/3/4/5 各造一次；stdout 与 stderr 分流；`--json` schema 可被脚本解析；非 TTY（cron / Hermes）路径；`--dry-run` 零写入；`--unique` 连跑两次；`--create` 新建文件与父目录；server 未启动报错；`--no-token` 实例；两个实例并存时消歧；登记表陈旧条目的提示。
- **端到端**：写入后核对落盘原文、索引汇总、Git 提交队列、界面 revision；真实目录用 `tools/verify-real-dir.ps1` 复核「零改写」。

实施与 review 验证（2026-11-09）：`cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo build --release` 三项全绿；另有两份端到端脚本（临时文件，不在仓库）以真实进程跑完全部断言，全绿。覆盖：退出码矩阵、`--dry-run` 零写入、`--unique` 跳过、`--create` 新建（普通笔记不套日记模板）、日程白名单与老笔记别名、CRLF / BOM / 混合行尾保持、H3 段尾、标题换行注入拒绝、段不存在时尾空格保留、代码块内标题不误命中、旧前端只传 `text` 零改动、索引 / revision / Git 提交队列、带 token 与 `--no-token`、多实例消歧与同根拒绝、陈旧条目过滤、登记表权限与落点、`--json` 错误走 stderr。

首轮 review 发现并修复：token 登记表回退路径可能落进笔记根、`--section` 可注入多行、混合行尾被整篇统一、H3 段尾判定、登记表非原子写与启动竞态、chunked 尺寸溢出、退出码与错误码映射、Windows `--dir` 归一化、同笔记根多实例并发写、日程显式标题破坏别名兼容、段不存在时抹掉尾空格、文档 token 口径冲突。

`pwsh -File tools/release-gate.ps1` 未执行：本机无 pwsh，未验证，需在 Windows 侧或装了 pwsh 的 CI 上补跑。设计文档已按实施结果更新状态。
