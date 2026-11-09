# NotesChain 发布说明与操作手册

适用版本：v0.1.1（对应 commit 见 `git log`）  
更新日期：2026-11-04

## 一、构建与分发

单文件可执行程序，前端与编辑器资源全部编译期内嵌。

| 平台 | 构建命令 | 产物 | 体积 |
| --- | --- | --- | --- |
| Windows x64 | `cargo build --release --locked` | `target\release\noteschain.exe` | 约 3.9 MB |
| Linux x64 | `cargo build --release --locked` | `target/release/noteschain` | 约 3.9 MB（首次 Linux 构建后复核） |

前端编辑器产物已随仓库提交，只有改过 `web/src/editor.js` 才需要重新打包：

```powershell
cd web; npm install; npm run build; cd ..
```

运行环境：Windows 10/11（x64）、Linux x64。**运行时唯一外部依赖是 git**（不装则版本历史停用，其余功能正常）；前端无构建产物时也能跑（编辑器回退为纯文本域）。

GitHub Actions 在推 `v*` tag 时自动构建并发布两个平台的压缩包与 sha256，见 `README.md` 的「发布」段。

## 二、启动与访问

```powershell
noteschain.exe --dir D:\notes --port 8090
```

| 参数 | 说明 |
| --- | --- |
| `--dir` | 笔记根目录（必填，不存在会报错退出） |
| `--port` | 端口，默认 8090，只绑 `127.0.0.1` |
| `--links` | 开启双链 [[目标]]（实验性，默认关闭） |
| `--no-git` | 关闭 Git 自动提交（同步盘等场景），其余功能保留 |

启动后终端打印一次带 token 的地址：

```
http://127.0.0.1:8090/?token=<64位随机串>
```

- 首次访问会用 token 换一个 HttpOnly + SameSite=Strict 的 Cookie，然后跳到干净地址
- token 每次启动重新生成，除进程内存外另存于用户级实例登记表（权限 600，不在笔记目录内），
  **重启后必须用新打印的地址重新打开**
- 不带 Cookie 访问任何接口都返回 401

## 三、日常使用

1. **首页**：上方日程（本日 / 本周 / 本月，默认本周），下方全部待办（未完成 / 已完成 / 全部），点勾即写回原文件并记完成时间
2. **左栏**：文件树（按年/月折叠，当前月默认展开）、标签树（可逐级折叠，显示笔记数与待办数）
3. **搜索**：`Ctrl+K` 或 `/` 聚焦；支持关键词 + 标签 + 状态过滤，代码命中默认折叠一行
4. **编辑器**：点文件即进入 CodeMirror 编辑态，停止输入 800ms 自动保存，`Ctrl+S` 立即保存；冲突时给「重新加载 / 强制覆盖」
5. **历史**：侧边栏「历史」看提交列表与 diff，可按文件恢复到某一版本
6. **今日**：一键打开当天日记（不存在时按模板自动新建）

## 四、数据与备份

- 笔记是**纯 Markdown 文件**，程序不引入私有格式
- 自动 `git init` 并生成默认 `.gitignore`（只忽略临时文件与系统文件，`assets/` 不忽略）
- 默认提交身份：用户自己配置过就用用户的，没有则临时用 `NotesChain <notes@local>`，**不修改你的 git 配置**
- **Git 自动提交防的是改错、删错，不防磁盘损坏**。界面常驻提示「仅本地版本历史，无远程备份」，请自行定期复制仓库或加远程

## 五、已知限制

| 限制 | 说明 |
| --- | --- |
| 不做图谱 / 关系视图 | 双链只做定位、跳转与反向链接 |
| 日程只认一种写法 | 必须 `## 今日日程` 段里 `HH:mm[-HH:mm] 名称`；其余进「未识别」诊断区 |
| 不支持全天 / 跨天 / 时区 | 所有时间按本机本地时间朴素解释 |
| 搜索不分词 | 子串匹配，中文按原文逐字；不做排序（固定按文件日期倒序 + 行号） |
| 标签不能批量改名 | 只能外部编辑器全局替换；空标签会置灰 |
| 同步盘未承诺可用 | OneDrive 等目录下 `.git` 可能与同步客户端冲突，建议不要启用自动提交 |
| 大文件 | `assets/` 下超 5 MB 只提示，不擅自忽略 |
| `/api/todos` 是 O(n) | 约 4 千条 104ms；数万条会变慢，已有 limit/offset 兜底 |

## 六、上线前检查清单

按顺序做完再发：

```powershell
# 发布前置（必须）：同步 bump Cargo.toml 与 Cargo.lock 的 version，
# 且即将打的 tag 必须等于 v<version>（CI 会校验，不一致直接失败）。
# 例：version = "0.1.2" → git tag v0.1.2

# 0) 一条命令跑完全部自动验证（构建 / 单测 / clippy / 冒烟 / 边界 / 平替预演 / 浏览器交互）
pwsh -File tools\release-gate.ps1

# 1) 单元测试
cargo test

# 2) 静态检查
cargo clippy --all-targets

# 3) 端到端冒烟（会在 .demo\smoke-notes 造语料）
pwsh -File tools\smoke.ps1

# 2.4) 边界场景（怪文件名 / 大文件 / GBK / .gitignore / 空目录）
pwsh -File tools\edge-check.ps1

# 2.5) 真实目录只读体检（不建仓库、不改文件）
pwsh -File tools\verify-real-dir.ps1 -Path "D:\notes" -Out .\体检报告.md

# 3.5) 真实浏览器交互（悬停预览、点击待办）——需要先起服务并把带 token 的地址传进去
# pwsh -File tools\browser-probe.ps1 -Url "http://127.0.0.1:8090/?token=..." -Hash "#/file/2026/11/2026-11-01.md"

# 4) 性能（可选，1000 文件语料）
node tools\gen-corpus.js .demo\perf-notes
#   然后按 docs\perf-report.md 的测法手工计时
```

手工确认项（无法自动化）：

- [ ] **用真实笔记目录启动**：目录结构、标签、待办全部识别；除 `.gitignore` 与 `.git` 外不改动任何已有文件
- [ ] **图片显示**：真实目录里的相对写法与根基准写法都能渲染（双轨解析）
- [ ] **git 提交**：编辑后 30 秒内产生一次本地提交，`git log` 可见 `auto: ...` 记录
- [ ] **无 git 降级**：临时把 git 从 PATH 移除启动，界面提示明确、其余功能正常
- [ ] **目标云桌面**：复测冷启动、空载内存、待办汇总耗时，记录机器配置与网络盘行为

## 七、故障排查

| 现象 | 处理 |
| --- | --- |
| 打开页面 401 | 服务重启过，token 已失效——用终端新打印的地址打开 |
| 顶部提示「未检测到 git」 | git 不在 PATH；版本历史停用，笔记功能不受影响 |
| 提示「目录位于同步盘内」/ 提交失败反复出现 | 建议把笔记目录移出同步盘，或在设置里关闭自动提交 |
| 中文文件乱码被跳过 | 该文件不是 UTF-8（可能是 GBK），启动日志会给 warnings，请转码后再用 |
| 图片不显示 | 检查引用路径是否在 notes 根内；`/api/asset` 对外链与越界路径会拒绝 |
| 待办点不动 | 页面提示「该待办已不存在或内容已变更」说明文件被外部改过，刷新页面即可 |

## 八、目录结构（开发）

```
src/            Rust 后端（api / index / markdown / schedule / todo_write / git / watch / state）
web/            前端（index.html / app.js / style.css / src/editor.js → dist/editor.js）
docs/           PRD、验收表、性能报告、模板
tools/          gen-corpus.js（语料生成）、smoke.ps1（端到端冒烟）
```
