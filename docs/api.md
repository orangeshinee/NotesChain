# NotesChain 接口文档

> 以 `src/api.rs` 的实现为准核对整理。NotesChain 是本地单机服务，默认只监听 `127.0.0.1`（默认端口 **8090**）。
> 本文覆盖全部 HTTP 接口：鉴权、文件、待办、日程、标签、搜索、模板、Git、双链、静态资源。

## 1. 基础约定

### 1.1 基址

```
http://127.0.0.1:8090
```

### 1.2 鉴权（PRD 9.1）

- 启动时生成随机 token，打印形如 `http://127.0.0.1:8090/?token=<64位hex>`。
- 首次访问：`GET /?token=<token>` 校验 query，服务器种一个 `HttpOnly; SameSite=Strict` 的 Cookie `nc_token`，然后 302 到 `/`。
- 之后所有请求（`fetch`、`<img>`、`<a download>`）靠浏览器自动携带 Cookie。
- **所有 `/api/*` 与静态资源接口**（`/app.js`、`/style.css`、`/editor.js`）都校验 Cookie；缺失或错误返回 `401 unauthorized`。
- 启动参数 `--no-token`：进程内 token 为空，**全部接口不鉴权**（仅建议个人独占机器使用）。
- token 只存在进程内存，重启即失效。

### 1.3 统一响应包络与错误码

成功：业务字段直接置于顶层。

失败：

```json
{ "error": { "code": "conflict", "message": "可直接展示的中文", "detail": {} } }
```

`code` 为稳定枚举，前端只判 `code`：

| HTTP | code | 场景 |
| --- | --- | --- |
| 400 | `bad_path` | 路径越界 / 含 `..` / 绝对路径 / 软链跳出根目录 |
| 400 | `bad_request` | 请求体字段缺失或格式非法 |
| 401 | `unauthorized` | token 缺失或错误 |
| 404 | `not_found` | 文件 / 资源不存在 |
| 409 | `conflict` | 整文件保存版本号不一致（`?force=true` 可强制覆盖） |
| 422 | `todo_relocate_failed` | 待办三级定位无法唯一确定目标行，禁止写入 |
| 500 | `internal` | 其它未分类错误 |
| 503 / 400 | `git_unavailable` | 未启用 git（`/api/git/status` 走 200 + 降级字段） |

### 1.4 版本号

文件版本号格式：`<mtime毫秒>-<sha256(规范化内容)前16位hex>`。读取文件时返回；保存/回滚时必须带回（待办回写仅作参考，走三级定位）。

### 1.5 路径约定

- 接口里的路径**相对笔记根**（`notes/`，旧布局则为项目根），统一 `/` 分隔，例如 `2026/11/2026-11-02.md`。
- 一律经后端 `resolve()` 规范化 + 根目录包含校验，越界即 `bad_path`。

---

## 2. 接口总览

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/` | 页面入口（`?token=` 首次引导种 Cookie） |
| GET | `/app.js` `/style.css` `/editor.js` | 内嵌前端静态资源（需鉴权） |
| GET | `/api/files` | 文件列表（日记/主题笔记分组） |
| GET | `/api/files/*path` | 读取单文件原文 |
| PUT | `/api/files/*path` | 整文件保存（版本校验） |
| DELETE | `/api/files/*path` | 删除（移入 `.trash/`） |
| POST | `/api/daily` | 确保当天日记存在 |
| GET | `/api/todos` | 全局待办汇总 |
| PATCH | `/api/todos` | 勾选 / 取消勾选（行级回写） |
| POST | `/api/todos/append` | 快速新增待办 |
| GET | `/api/tags` | 标签树 |
| GET | `/api/search` | 全文搜索 |
| POST | `/api/notes/new` | 新建笔记（模板 / 日记） |
| GET | `/api/templates` | 列出模板 |
| GET | `/api/schedule` | 日程视图（today/week/month） |
| POST | `/api/schedule/append` | 快速新增日程 |
| POST | `/api/lines/append` | 通用追加一行到指定标题 |
| GET | `/api/config` | 运行配置 / 提示信息 |
| GET | `/api/revision` | 索引修订号（前端轮询） |
| POST | `/api/index/rebuild` | 重建内存索引 |
| GET | `/api/git/status` | Git 状态 |
| GET | `/api/git/log` | 提交历史 |
| GET | `/api/git/show` | 单次提交 patch |
| POST | `/api/git/restore` | 按文件回滚到某提交 |
| GET | `/api/links` | 某文件的出链（双链） |
| GET | `/api/backlinks` | 某文件的反向链接 |
| GET | `/api/migration/todo` | 历史 `todo:` 写法清单 |
| GET | `/api/asset` | 图片 / 附件静态服务 |

---

## 3. 文件

### GET /api/files

返回文件树数据，日记按「年 → 月」两级分组，其余归入 `note` 组。

```json
{
  "groups": [
    { "kind": "diary", "years": [
      { "year": "2026", "months": [
        { "month": "11", "entries": [
          { "path": "2026/11/2026-11-02.md", "mtime": 1762000000000, "size": 512, "title": "2026/11/2026-11-02.md", "todos": 2 }
        ] }
      ] }
    ] },
    { "kind": "note", "entries": [ { "path": "raw/KPI口径.md", "mtime": 0, "size": 0, "title": "raw/KPI口径.md", "todos": 1 } ] }
  ],
  "warnings": []
}
```

`FileItem`：`path`、`mtime`(毫秒)、`size`、`title`、`todos`(未完成待办数)。

### GET /api/files/*path

```json
{ "path": "2026/11/2026-11-02.md", "content": "# 2026-11-02\n…", "eol": "lf", "version": "1762000000000-abcdef0123456789" }
```

- `content` 统一去 BOM、坏行尾（换行统一为 `\n`）。
- `eol`：`lf` | `crlf`（原文件行尾，保存时按此还原）。
- 404 `not_found` 文件不存在。

### PUT /api/files/*path

请求体：

```json
{ "content": "全文", "version": "读取时拿到的版本号", "eol": "lf" }
```

- Query：`?force=true` 跳过版本校验强制覆盖（用于冲突弹窗的「强制覆盖」）。
- 成功：`{ "path": "...", "version": "新版本号" }`
- 版本不一致且非 force：`409 conflict`。

### DELETE /api/files/*path

仅 `.md`。不硬删，移入 `<root>/.trash/<时间戳>-<文件名>`。

```json
{ "path": "2026/11/2026-11-02.md", "trashedTo": "F:/MyNotes/.trash/20261102-213000-2026-11-02.md" }
```

### POST /api/daily

确保当天日记 `YYYY/MM/YYYY-MM-DD.md` 存在；不存在则按日记模板新建（幂等）。

```json
{ "path": "2026/11/2026-11-02.md", "created": true, "content": "...", "version": "..." }
```

---

## 4. 待办

### GET /api/todos

Query：`tag`（含子标签）、`status`（`open` | `done` | `all`，默认 `open`）、`limit`（默认 5000，上限 50000）、`offset`。

```json
{
  "total": 12, "offset": 0, "returned": 12,
  "items": [
    {
      "path": "2026/11/2026-11-02.md",
      "date": "2026-11-02",
      "line": 12,
      "indent": 0,
      "text": "写周报 #工作",
      "raw_line": "- [ ] 写周报 #工作",
      "tags": ["工作"],
      "version": "1762000000000-abcdef0123456789",
      "done": false,
      "done_at": null,
      "priority": 1
    }
  ]
}
```

- `raw_line` = 去掉行尾完成标记后的整行（含缩进与 `- [ ]`），是 `PATCH` 回写的定位依据。
- `priority` 为 `!pN` 的 N（0–9），无则 `null`。
- `date` 为日记日期，非日记 `null`。

### PATCH /api/todos

勾选 / 取消勾选，后端按「行号 → 全文唯一 → 就近」三级定位回写。

请求体：

```json
{ "path": "2026/11/2026-11-02.md", "line": 12, "raw_line": "- [ ] 写周报 #工作", "version": "可选", "done": true }
```

- 成功：`{ "path": "...", "line": 12, "version": "新版本号" }`
- 定位失败：`422 todo_relocate_failed`（禁止写入）。

### POST /api/todos/append

追加一条 `- [ ] …`。请求体（除 `text` 外均可选）：

```json
{ "text": "买牛奶 #生活", "path": "2026/11/2026-11-09.md", "section": "## 待办",
  "create": null, "unique": false, "dry_run": false }
```

- `path`：缺省今天的日记；`section`：缺省 `## 待办`，接受 `## 待办` 或 `待办`。
- `create`：不传时沿用旧行为（文件不存在按模板新建）；**显式 `false` 时返回 `404 not_found`**，CLI 默认传 `false`。
- `unique`：目标段内已有完全相同行时返回 `action: "skipped"`，不写盘。
- `dry_run`：只返回将要插入的位置，零写入。

成功：`{ "ok": true, "action": "inserted|skipped|dry_run", "path": "...", "section": "...", "line": 18, "text": "- [ ] 买牛奶 #生活", "version": "..." }`

---

## 5. 标签 / 搜索

### GET /api/tags

统一的标签树（正文 `#标签` 与 frontmatter `tags` 合并索引）。

```json
{
  "tree": [
    { "name": "工作", "path": "工作", "notes": 3, "todos": 2,
      "children": [ { "name": "供应链", "path": "工作/供应链", "notes": 1, "todos": 1, "children": [] } ] }
  ]
}
```

`TagNode`：`name`、`path`、`notes`(笔记数)、`todos`(未完成待办数)、`children`。

### GET /api/search

Query：

- `q`：关键词（子串，英文不分大小写）。空 q 时按标签筛选，返回文件级结果。
- `tag`：按标签过滤（含子标签）。
- `status`：`all`(默认) | `open` | `done`。
- `scope`：`all`(默认) | `text`（排除代码块命中）。
- `limit`：默认 200，上限 1000。

```json
{
  "total": 1,
  "items": [
    { "path": "2026/11/2026-11-02.md", "line": 12, "snippet": "- [ ] 写周报 #工作",
      "kind": "todo", "before": "…", "after": "…", "done": false }
  ]
}
```

- `kind`：`text` | `todo` | `code`；只有标签、无关键词时 `kind` 为 `file`，`line` 为**命中标签的那一行**、`snippet` 为该行文本。

---

## 6. 新建 / 模板

### POST /api/notes/new

请求体（字段均可选，按组合决定行为）：

```json
{ "title": "周报", "path": "raw/周报.md", "type": "note", "template": "meeting", "date": "2026-11-02" }
```

- `path`：直接指定相对路径（优先）。
- 否则 `type=diary` 或带 `date` → 建 `YYYY/MM/YYYY-MM-DD.md`（日记模板）；否则用 `title` 建 `raw/<title>.md`（普通笔记模板）。
- `template`：使用 `<root>/templates/<name>.md` 自定义模板。

成功：`{ "path": "...", "type": "diary|note", "content": "...", "version": "..." }`；同名已存在返回 `409 conflict`。

### GET /api/templates

```json
{ "dir": "F:/MyNotes/templates", "templates": ["daily", "meeting", "note"] }
```

模板占位符：`{{TITLE}}` `{{DATE}}` `{{DATETIME}}` `{{TAGS}}`。

---

## 7. 日程

### GET /api/schedule

Query：`range` = `today` | `week`(默认) | `month`。

```json
{
  "range": "week", "from": "2026-11-02", "to": "2026-11-08",
  "days": [
    {
      "date": "2026-11-02", "weekday": "周一", "today": true,
      "path": "2026/11/2026-11-02.md",
      "items": [ { "start": "09:30", "end": "10:00", "title": "晨会", "tags": [], "place": "线上", "line": 5 } ],
      "unrecognized": ["记得带充电器"],
      "suspected": [29]
    }
  ]
}
```

- 仅有日记文件的日子才出现在 `days`（当天无文件也会给一个空 `today` 条目）。
- `items`：`start` `end`(可空) `title` `tags` `place`(可空) `line`。
- `unrecognized`：日程段内无法解析的行原文。
- `suspected`：仅 `today` 返回，段外「像时间」的行号。

### POST /api/schedule/append

追加一条 `- HH:mm …`。请求体字段同 `POST /api/todos/append`：`text` / `path` / `section` / `create` / `unique` / `dry_run`。

- `path` 缺省今天的日记；`section` 缺省走候选列表 `["## 日程", "## 今日日程"]`，兼容模板改名前的老笔记。
- `section` 只允许这两个标题，传别的返回 `400 bad_request`：日程解析器只认这两个段，写到别处会静默不可见。

成功体同 `POST /api/todos/append`。

### POST /api/lines/append

通用追加：把一行原文写到指定文件的指定标题下，不补任何前缀。

```json
{ "text": "和财务确认口径", "path": "raw/周报.md", "section": "## 记录" }
```

`section` **必填**（不传返回 `400 bad_request`）；其余字段与上面两个接口一致。段不存在时自动新建该段。

---

## 8. 配置 / 索引

### GET /api/config

```json
{
  "links": false,
  "duplicateNames": [ { "name": "A", "paths": ["raw/A.md", "raw/sub/A.md"] } ],
  "largeAssets": [ { "path": "assets/big.zip", "sizeMb": 12.3 } ],
  "syncDrive": "OneDrive",
  "noToken": false,
  "commitDebounce": 300,
  "commitForce": 1800,
  "root": "F:/MyNotes"
}
```

- `links`：是否启用双链。
- `duplicateNames`：`raw/` 下同名裸名冲突。
- `largeAssets`：`assets/` 下超过 5MB 的文件。
- `syncDrive`：检测到的同步盘名（无则 `null`）。
- `noToken` / `commitDebounce` / `commitForce`：对应启动参数（秒）。

### GET /api/revision

```json
{ "revision": 42 }
```

前端每 5 秒轮询；变化则刷新侧边栏，必要时重载当前文件。

### POST /api/index/rebuild

```json
{ "rebuilt": true, "files": 1000, "warnings": [] }
```

---

## 9. Git

### GET /api/git/status

```json
{
  "available": true, "disabled_by_user": false, "is_repo": true,
  "repo_root": "F:/MyNotes", "last_commit": "2026-11-02 21:30",
  "pending_files": 2, "last_error": null, "disabled": false,
  "note": "仅本地版本历史，无远程备份"
}
```

Git 不可用时 `available:false` 并给降级字段，不报错。

### GET /api/git/log?limit=80

```json
{ "available": true, "commits": [ { "hash": "<40位>", "short": "<8位>", "time": "2026-11-02 21:30", "subject": "auto: 2026-11-02 21:30 (2 files)" } ] }
```

`limit` 默认 80，上限 500。不可用返回 `{ "available": false, "commits": [] }`（可能带 `reason`）。

### GET /api/git/show?hash=<hash>

```json
{ "hash": "<hash>", "patch": "diff --git a/... b/...\n..." }
```

失败：`400 bad_request` / `400 git_unavailable`。

### POST /api/git/restore

请求体 `{ "hash": "<hash>", "path": "2026/11/2026-11-02.md" }`，把某文件恢复到该提交版本（会覆盖当前内容）。

成功：`{ "path": "...", "version": "..." }`

---

## 10. 双链 / 迁移

> 双链为 P1、默认关闭（`--links` 开启）。关闭时相关接口仍可调用，但前端不展示。

### GET /api/links?path=<文件>

某文件的出链（该文件中出现的所有 `[[目标]]` 及解析结果）。

```json
{ "path": "raw/A.md", "total": 2, "items": [
  { "target": "KPI口径", "resolved": "raw/KPI口径", "line": 3 },
  { "target": "不存在的", "resolved": null, "line": 8 }
] }
```

### GET /api/backlinks?path=<文件>

反向链接（哪些文件链向它）。

```json
{ "path": "raw/KPI口径", "total": 1, "items": [ { "path": "2026/11/2026-11-02.md", "line": 15, "text": "…[[KPI口径]]…" } ] }
```

### GET /api/migration/todo

历史 `todo:` 写法清单（只读，不改写）。

```json
{ "total": 2, "items": [ { "path": "...", "line": 9, "text": "todo: 老写法" } ] }
```

---

## 11. 静态资源

### GET /api/asset?path=<引用路径>&from=<当前文件>

图片 / 附件服务。双轨解析：先按 `from` 文件所在目录，再按 `assets/` / `notes` 根 / 项目根；越界或不存在返回 `404`，拒绝 `http(s)://`、`data:`。

按扩展名返回 MIME（png/jpg/gif/webp/svg/bmp/pdf/txt/md，其余 `application/octet-stream`），`Cache-Control: no-cache`。

### GET /app.js · /style.css · /editor.js

编译期内嵌的前端资源，`Cache-Control: no-cache`（升级后不会新旧混搭）。需通过鉴权。

---

## 12. 说明

- 本文与代码同步维护：接口签名变化时请一并更新此文件与 `docs/noteschain-prd-v05.md` 的 API 清单。
- 所有写接口遵守固定顺序：`resolve()` → 按文件锁 → 原子写 → 更新索引 → `git.notify()` → `revision+1`。
