# NotesChain 代码审计报告（第二轮）

- 审计日期：2026-11-03
- 审计基线：`979c57554f925f33a11d1d4c6cdd8e1b3761f8db`（`master`，tag `v0.1.1`）
- 上一轮基线：`1856b3e0337dce39741203578a752d1cea69292e`（见 `docs/code-review-2026-11-01.md`）
- 审计范围：`src/` 全部 9 个 Rust 模块（4051 行）、`web/app.js`（2571 行）、`web/src/editor.js`、前端 Markdown 渲染路径、CI 与发布配置
- 审计方式：只读代码审计 + 临时目录实跑复现 + 项目自带验证命令
- 与上一轮的关系：上一轮已列出 H1–H9 / M1–M11 / L1–L4 并当晚修复了大部分。**本轮的重点是复验那批修复在 v0.1.1 上是否真的落地，并补齐上一轮未覆盖的面**（架构分层、异步阻塞、资源生命周期、响应头安全）。不重复上一轮已经确认的优点。

> 本报告只做审计与复现，未修改任何业务代码、测试脚本或笔记数据。复现全部在 `/tmp` 临时目录进行，已清理。

---

## 1. 总体判断

上一轮的问题是"跨层不变量没真正做到"，这一轮看下来，**那些不变量基本被认真修了**：版本号统一走 `canonical_text` 归一化、监听与扫描共用同一套忽略规则、git 恢复改成多候选试解、待办回写按行保留行尾——都能在代码里找到对应实现，且补了针对性单测（测试数从 26 增至 33）。

但复验暴露出一个更要命的问题：**上一轮标记为 ✅ 已修的 M5（路径软链接防护）实际只修了一半，而且引入了新的可用性缺陷——`notes/` 一旦是软链接，全部笔记可读不可读、可列不可开**。这不是推断，是本轮实测复现的。

除此之外，代码质量本身是干净的（`clippy` 零警告、几乎零 `unwrap`、注释与 PRD 引用齐备）。剩下的问题集中在三个方向：**单文件承载过多**、**异步路径上的阻塞与资源泄漏**、**响应头缺失**。

---

## 2. 验证结果与证据边界

| 验证 | 结果 | 说明 |
| --- | --- | --- |
| `cargo test` | 通过 | 33 passed，0 failed（上一轮基线为 26） |
| `cargo clippy --all-targets` | 通过 | 0 warnings / 0 errors，满足 CI 的零警告要求 |
| `cargo build` | 通过 | debug 构建成功，用于下述复现 |
| 软链接 `notes/` 目录：启动 + 列表 + 读取 | **复现缺陷** | 列表 `200` 且正常收录 1 个文件；读取 `400 bad_path 路径越界` |
| 对照组：真实目录（非软链接）同样操作 | 通过 | 读取 `200`，返回完整内容与 version |
| `/api/asset` 响应头 | **确认缺失** | 仅 `content-type` + `cache-control`，无 CSP、无 `X-Content-Type-Options` |
| `/api/asset` 取 `.svg` | 确认 | `content-type: image/svg+xml`，无 CSP/nosniff（浏览器行为未实测） |
| `tools/release-gate.ps1` | **未验证** | PowerShell 脚本，本机无 pwsh，无法运行 |
| `tools/*.js` 语料生成器护栏 | **未验证** | 本轮未运行（上一轮 H8 相关） |
| 前端交互时序（H1/H2/H3、M8/M9） | **未验证** | 本轮未跑浏览器层，沿用上一轮结论 |

复现环境：`/tmp/nc-link/notes -> /tmp/nc-real`（软链接），`--no-token --no-git --port 8099`；对照 `/tmp/nc-plain/notes`（真实目录）。测试结束已 `pkill -x noteschain` 并删除全部临时目录，无残留进程。

---

## 3. 上一轮问题复验（基线 `1856b3e` → `979c575`）

状态说明：✅ 代码层面确认已落地 / ⚠️ 部分落地或有残留 / ❌ 未修 / ⬚ 本轮未验证。

| 编号 | 问题 | 状态 | 依据 |
| --- | --- | --- | --- |
| H4 | BOM/CRLF 的 version 口径不一致 | ✅ | `index.rs:209-212` `version_of_text` 先过 `canonical_text` 再哈希；`index.rs:370-378` 有单测锁住 BOM/CRLF/LF 三写法同版本 |
| H5 | 旧布局下 `.trash` 被重新收录 | ✅ | `index.rs:315` `Ignored` 默认 `dirs` 已含 `.trash`；`index.rs:44` warnings 已合并 |
| H6 | 监听不应用忽略规则、删除不入索引 | ✅ | `watch.rs:40` 与扫描共用 `Ignored::load`，`watch.rs:75` 显式排除 `.git`/`.trash`，`watch.rs:103` 走 `Index::remove` |
| H7 | 标准布局 git 恢复路径拼接错误 | ✅ | `api.rs:1176-1216` 按「仓库相对 / 项目根相对 / notes 相对」生成候选并逐个 `git show` 试解，含嵌套仓库前缀剥离 |
| H8 | 语料生成器可删任意目录 | ⬚ | 本轮未运行 `tools/*.js` |
| H9 | 发布闸门可零断言通过 | ⬚ | 本机无 pwsh，无法验证 |
| M1 | 混合 EOL 文件被整篇改写 | ✅ | `api.rs:553-567` 用 `split_inclusive('\n')` 按段保留各行自己的行尾 |
| M2 / M3 / M6 | 锁键不统一、创建无锁内二次检查、缺 `.md` 校验 | ✅ | 写接口统一 `ensure_markdown` + 锁键用 `st.relative(&abs)` |
| M4 | 重建索引与写入并发不协调 | ⚠️ | `state.rs:86-100` 引入 `revision` epoch 乐观重试（最多 3 次）。降低了概率，但 epoch 检查与 `index.write()` 之间仍有窗口；连续三次撞车时是「保留旧索引」而非重试成功，属于退让而非解决 |
| M5 | 软链接 / 缺失父目录路径防护 | ❌ | **本轮实测：安全面达成（不再越界写入），但可用性面新引入缺陷——`notes/` 为软链接时所有合法路径也被判越界**，详见 F1 |
| M7 | `git commit` 无 pathspec 连带父仓库 | ✅ | `git.rs:253-254` 提交带 `-- <rel>`，rel 由 `strip_prefix(repo_root)` 计算 |
| M8 / M9 | 前端请求代际、卸载保护 | ⬚ | 未跑浏览器层 |
| M10 / M11 | `.gitignore` warnings、断言过宽 | ⬚ | 未验证 |
| L1 | 已带 Cookie 时 `?token=` 仍重种 Cookie | ✅ | `api.rs:192-196` `has_valid_cookie` 为真时直接重定向到 `/` |
| L2 / L4 | 日程段标题、空标题日程 | ⬚ | 未验证 |
| L3 | 日记路径只校验形状，`2026-02-31` 也算日记 | ⚠️ | **半修**：`index.rs:258-261` 已加 `NaiveDate::parse_from_str` 并回校年月；但 `api.rs:39-53` 的 `diary_date_from_path` 未同步，仍只做字符串形状判断。详见 F3 |

**复验小结**：H 系列与 M1/M2/M3/M6/M7、L1 确认落地；M4 部分落地；**M5 与 L3 存在残留**，其中 M5 是本轮实测出的新问题。

---

## 4. 本轮新发现

### [CRIT] F1 —— `notes/` 为软链接时，全部笔记「能列不能读」

位置：`src/state.rs:58-62`（`notes_root` 构造）、`src/state.rs:126-151`（`resolve` 的包含校验）

触发条件：`--dir` 指向的目录下，`notes` 是软链接（指向别处的真实目录）。

实测结果：

```
启动：  笔记目录：/tmp/nc-link/notes
        索引：1 个 Markdown 文件，1 条未完成待办     ← 扫描正常收录
GET /api/files                  → 200，正常返回 1 个文件
GET /api/files/2026-11-01.md    → 400 {"code":"bad_path","message":"路径越界"}
对照组（真实目录 /tmp/nc-plain） → 200，正常返回正文与 version
```

why：`resolve()` 先把路径向上找到最近的已存在祖先做 `canonicalize`（`state.rs:129-143`），软链接会被解析成真实目标路径；随后 `state.rs:148` 拿它去 `starts_with(&self.notes_root)`。而 `notes_root` 是 `root.join("notes")` 拼出来的，**从未 canonicalize**（`root` 自己规范化过，两个子目录漏了）。真实路径与未解析的软链接路径永远对不上前缀，于是所有路径一律判为越界。

同时 `Index::scan` 走 `WalkDir`（`index.rs:46`）能跟随起点软链接正常收录，于是出现「列表看得见、点开就 400」的内部不一致。

影响面：扫描、待办汇总、搜索都正常，唯独单文件读写（打开、保存、删除待办回写）全部 400。用户会以为程序坏了，且从列表看不出任何异常。软链接在「笔记放 NAS/同步盘、本地软链进来」的场景下并不罕见。

与 M5 的关系：M5 的安全目标（不写到根目录外）确实达成了，代价是**误伤了合法路径**。属于修偏，不是没修。

fix：构造 `notes_root` / `assets_root` 时同样做 `canonicalize + simplify`：

```rust
let notes_root = if root.join("notes").is_dir() {
    simplify(root.join("notes").canonicalize().unwrap_or_else(|_| root.join("notes")))
} else { root.clone() };
```

并补一条软链接布局的回归测试（建临时 symlink 目录 → 断言 `resolve` 成功），避免再次回归。

---

### [STRUCT] F2 —— `src/api.rs` 单文件 1855 行，承载 22 个路由与全部业务

位置：`src/api.rs:55-88`（路由表）及全文

why：同一个文件里同时装着 HTTP 层（`auth` / `cookie_token`）、文件读写（`read_file` / `save_file` / `delete_file`）、待办与日程、git 子进程封装（`1001-1264`）、模板渲染（`28-36`）、搜索与双链。新增接口只能继续往里堆，review 面随功能线性膨胀，`clippy` 与单测也挤在同一文件末尾（`1817-1855`）。

fix：按域拆 `files.rs` / `todos.rs` / `schedule.rs` / `git_api.rs` / `search.rs` / `templates.rs`，`router()` 只保留组装。`web/app.js` 2571 行同理（路由 + Markdown 渲染 + diff + 日程 + 设置全在一个文件）。

---

### [STRUCT] F3 —— 日记判定两份实现，严格程度不同

位置：`src/api.rs:39-53` `diary_date_from_path` vs `src/index.rs:239-264` `diary_from_path`

why：两个函数做的是同一件事（判断相对路径是否为日记），但 api 版只校验形状（`api.rs:48`：长度 10 + 前缀匹配），index 版会 `NaiveDate::parse_from_str` 并回校年月（`index.rs:258-261`）。于是 `2026/02/2026-02-31.md` 这种不存在的日期，在 `ensure_daily`（`api.rs:695-696`）里算日记、在索引分组里不算，两边结论不一致。这正是上一轮 L3 只修了一半的原因。

fix：删掉 `api.rs` 的副本，`ensure_daily` 改调 `index::diary_from_path`（改为 `pub`）。单一真相源。

---

### [INCR] F4 —— `Git::status()` 在 async 里同步跑 `git status`，阻塞 Tokio worker

位置：`src/git.rs:114-123` → `count_pending`（`git.rs:274-286`）

why：同文件里 `git_log`（`1006`）、`git_show`（`1064`）、`git_restore`（`1226`）都规规矩矩用了 `spawn_blocking`，唯独 `status()` 里的 `count_pending` 是直接同步 `Command::output()`。前端轮询 `/api/git/status` 时，每次都在 runtime 线程上干等子进程返回；仓库大或磁盘慢时是几十到几百毫秒的停顿。

fix：`count_pending` 也包一层 `spawn_blocking`，与其余三个 git 接口保持一致。

---

### [INCR] F5 —— `file_locks` 只增不减

位置：`src/state.rs:102-108`

why：`lock_for` 用 `entry().or_insert_with` 插入，全局没有任何 `remove` / `retain`（已 grep 确认）。删除或重命名笔记后，对应的 `Arc<Mutex<()>>` 永久留在 HashMap 里。单次泄漏极小，但是单向增长，长跑进程会持续累积。

fix：删除路径（`delete_file`、`Index::remove`）时顺手清理，或改用 `Weak` + 定期清扫。

---

### [INCR] F6 —— `serve_bytes` 缺 CSP 与 `nosniff`，SVG 以同源 `image/svg+xml` 返回

位置：`src/api.rs:1784-1807`

实测响应头：

```
HTTP/1.1 200 OK
content-type: image/svg+xml      ← 取 .svg 时
cache-control: no-cache
（无 Content-Security-Policy，无 X-Content-Type-Options）
```

why：直接导航到 `/api/asset?path=x.svg` 会在同源上下文里渲染 SVG，而 SVG 可内嵌脚本，进而能调用本服务的 `/api/*`。本地单用户、只绑回环地址的前提下危害有限，但修复成本几乎为零。

fix：加 `Content-Security-Policy: default-src 'none'` 与 `X-Content-Type-Options: nosniff`。（浏览器实际执行脚本的行为本轮未实测，属推断；响应头缺失是实测。）

---

### [INCR] F7 —— 全部笔记正文常驻内存，搜索走线性扫描

位置：`src/index.rs:26`（`FileEntry.content`）、`index.rs:171-199`（`load_file`）

why：`content` 字段保留了每篇全文（注释说明是为全文搜索服务），`scan` 逐篇读盘进内存；搜索再遍历全部文件做子串匹配。笔记量上去后，内存占用与单次搜索耗时都是线性增长，与项目「极度轻量」的定位有张力。

fix：正文惰性加载，或建一个最小倒排；至少给搜索加上限与提前退出。

---

### [INCR] F8 —— `git.rs`（377 行）与 `watch.rs`（114 行）零单测

why：33 个测试集中在 `markdown` / `index` / `todo_write` / `schedule` / `state` / `api`，偏偏两条最容易出错的异步路径——自动提交防抖（`git.rs:146-206` 的 debounce/force 双超时）与文件监听——完全没有覆盖。上一轮第 5 节列的测试缺口偏「数据安全行为」，未点名这两处。

fix：先给防抖时序和监听路径过滤补纯函数/集成测试，git 侧可用临时仓库做真实 commit → log → restore 链路测试。

---

### [INCR] F9 —— 验证入口只有 PowerShell，本机离线构建会失败

位置：`tools/release-gate.ps1`、`.github/workflows/{ci,release}.yml`（均 `runs-on: windows-latest`）

why：README 明写工具链是 `Rust stable (MSVC) + VS 2022`，CI 只跑 Windows。我在 Linux 上 `cargo test` / `cargo clippy` 全绿，但这不在 CI 的验证面内，跨平台行为无保障。另外本机 crates 缓存缺 `chrono 0.4.45`（`Cargo.lock` 锁定版本），`cargo test --offline` 会解析失败，首次构建必须联网。

fix：若发布目标就是 Windows-only，保持现状并在 README 写明；若要跨平台，补 ubuntu/macos 到 CI 矩阵。

---

## 5. 复验确认做得好的地方

不与上一轮第 7 节重复，只列本轮读码时确认到位、且实现得干净的点：

- **版本号口径归一化的实现很正确**：`version_of_text` 内部先过 `canonical_text` 再哈希（`index.rs:209-212`），GET/PUT/PATCH/索引同一口径，并用单测（`index.rs:370-378`）把 BOM / CRLF / LF 三种写法锁成同一版本。这个坑极易踩，踩了就是「保存两次必报冲突」。
- **待办回写按段保留各行行尾**（`api.rs:553-567`）：用 `split_inclusive('\n')` 而非全文 CRLF 转换，混合 EOL 文件只改目标行。
- **重建索引的 epoch 乐观重试**（`state.rs:86-100`）：虽不是彻底的读写协调（见 M4），但把「保存与重建交错丢索引项」从必然降成了小概率，且失败时选择保留旧索引而非写入坏数据。
- **git 子进程参数零注入面**：所有 `Command` 参数来自内部计算，用户提交的 hash 走 `is_ascii_hexdigit` + 长度校验（`api.rs:1060-1062`、`1170-1172`）。

---

## 6. 评分

| 轴 | 分 | 说明 |
| --- | --- | --- |
| Architecture | 6.5 | 模块切分清楚，但 `api.rs` 单文件扛 22 个路由与全部业务；日记判定存在两份实现 |
| Code Quality | 7.5 | 命名与注释扎实（每模块有 `//!` 头与 PRD 引用），几乎零 `unwrap`（全库 3 处且均在测试），33 个单测命中关键解析 |
| Engineering | 6.5 | CI + clippy 零警告 + tag 驱动发布齐备；但存在「标记为已修实际残留」（M5、L3），验证入口只覆盖 Windows |
| Perf & Risk | 6.0 | 软链接导致核心读写不可用（实测）、`git status` 阻塞 worker、锁泄漏、索引全量常驻内存、响应头缺 CSP |
| **Overall** | **6.6** | 上一轮的高危项基本认真修了；扣分主要在复验发现的残留与新增的异步/资源问题 |

---

## 7. Top 3 最高杠杆

1. **修 `notes_root` / `assets_root` 的 canonicalize（F1）** —— 实测确认的核心可用性缺陷，改动约 5 行，必须补一条软链接布局的回归测试锁住它。
2. **拆 `api.rs`，顺手删掉日记判定的副本（F2 + F3）** —— 唯一的结构性债，越晚越贵；F3 同时也是上一轮 L3 的另一半。
3. **`git status` 走 `spawn_blocking`，并给 `git.rs` / `watch.rs` 补测试（F4 + F8）** —— 异步路径目前没有任何测试兜底，出问题只能靠用户现场反馈。

---

## 8. 声明

- 本报告只做只读审计与临时目录复现，未修改业务代码、测试、配置或笔记数据。
- 复现目录均在 `/tmp` 下创建并在结束后删除，进程已确认退出，无残留。
- 标注「未验证」的项（H8/H9、M8–M11、L2/L4、PowerShell 闸门、浏览器交互）不在本轮证据范围内，沿用上一轮结论。
- 软链接缺陷与 asset 响应头为本轮实测；SVG 脚本执行、锁泄漏的长期影响、搜索性能为读码推断，已在文中标注。
