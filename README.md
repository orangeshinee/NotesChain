# NotesChain

极度轻量、本地优先的 Markdown 笔记工具。每天一个文件，保存即落盘，变更进 Git 提交队列——`Chain` 指的就是这条「文件 → 保存 → Git 提交 → 历史版本」的链。

需求文档：[docs/noteschain-prd-v05.md](docs/noteschain-prd-v05.md)（v0.4 存档于 [docs/archive/](docs/archive/)）。

## 当前状态

已跑通的链路：**扫描 → 日程/待办/标签 → 首页 → 编辑器保存 → Git 队列**。

已完成：

- 目录扫描（跳过 `.git` 与 `.gitignore` 命中项），日记文件判定（路径为主、内容兜底）
- 行级 Markdown 解析：代码块围栏状态机、待办提取、标签提取、完成标记剥离
- 首页：日程区（本日 / 本周 / 本月，默认本周）+ 全部未完成待办表（事项 / 文件 / 标签）+ 顶部快速添加待办（回车即写入今天）
- 待办勾选回写：三级定位（行号快路径 → 全文唯一匹配 → 就近匹配 → 拒绝写入），原子写入，按文件串行化
- 每日文件按模板新建（`docs/templates/daily-note.md`，编译期内嵌）
- 侧边栏：文件树（按年 / 月折叠）、标签树（含笔记数与未完成待办数）
- 全文搜索：子串匹配、标签过滤、状态过滤（全部 / 未完成 / 已完成）、命中高亮与跳转定位
- 编辑器：CodeMirror 6 + Markdown，L1 Live Preview，800ms 自动保存，冲突提示（重新加载 / 强制覆盖）
- 文件监听：外部编辑后索引与界面自动更新（前端 5 秒轮询修订号，编辑中不覆盖用户内容）
- Git：自动 init、默认 `.gitignore`、30 秒防抖自动提交、状态接口
- 启动 token + Cookie 鉴权、路径穿越防护

未做（按 PRD 排期）：

- Git 历史浏览 / diff / 回滚 UI（P1）
- 静态资源服务（图片目前显示为占位，`/assets/*` 未实现）
- 双链解析与反向链接面板（P1，默认关闭）
- 双链解析与反向链接面板（P1，默认关闭）

## 前端构建

编辑器用 CodeMirror 6，需要打包一次（产物已提交，改编辑器源码后重新构建）：

```powershell
cd web
npm install
npm run build      # 输出 web/dist/editor.js，编译期内嵌进二进制
```

`web/index.html`、`web/app.js`、`web/style.css` 是手写的，不需要构建。

## 运行

> 需要在自己机器上做一次完整验收，照着 [docs/verify.md](docs/verify.md) 走即可。

```powershell
cargo run -- --dir D:\notes --port 8080
```

启动后终端会打印带 token 的地址，形如：

```
http://127.0.0.1:8080/?token=<64位随机串>
```

首次访问会在浏览器种下 HttpOnly Cookie 并跳转到干净地址；token 只存在于本次进程，重启后需要用新地址重新打开。

参数：

| 参数 | 说明 |
| --- | --- |
| `--dir` | 笔记根目录（必填） |
| `--port` | 监听端口，默认 8080；只绑 `127.0.0.1` |

## 开发

```powershell
cargo test      # 17 个单元测试：解析、定位、路径防护
cargo build --release
```

代码结构：

| 文件 | 职责 |
| --- | --- |
| `src/markdown.rs` | 行级解析：围栏、待办、标签、完成标记 |
| `src/index.rs` | 内存索引、文件类型判定、版本号 |
| `src/todo_write.rs` | 三级定位、行改写、原子写入 |
| `src/state.rs` | 根目录解析、token、按文件写锁 |
| `src/api.rs` | HTTP 接口与鉴权 |
| `web/index.html` | 极简首页（原生 JS，无构建步骤） |

## 环境

Rust stable（MSVC 工具链）+ VS 2022 Build Tools。crates 走 rsproxy 镜像，配置见 `~/.cargo/config.toml`。
