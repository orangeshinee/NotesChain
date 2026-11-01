# NotesChain

极度轻量、本地优先的 Markdown 笔记工具。每天一个文件，保存即落盘，变更进 Git 提交队列——`Chain` 指的就是这条「文件 → 保存 → Git 提交 → 历史版本」的链。

需求文档：[docs/noteschain-prd-v05.md](docs/noteschain-prd-v05.md)（v0.4 存档于 [docs/archive/](docs/archive/)）。

## 当前状态

竖切（v0.1）已完成：**扫描 → 待办提取 → 首页勾选 → 三级定位回写** 这条 P0 主链路端到端跑通。

已完成：

- 目录扫描（跳过 `.git` 与 `.gitignore` 命中项），日记文件判定（路径为主、内容兜底）
- 行级 Markdown 解析：代码块围栏状态机、待办提取、标签提取、完成标记剥离
- 首页待办汇总与排序（日记按日期倒序在前，`raw/` 等无日期来源在后）
- 待办勾选回写：三级定位（行号快路径 → 全文唯一匹配 → 就近匹配 → 拒绝写入），原子写入，按文件串行化
- 每日文件按模板新建（`docs/templates/daily-note.md`，编译期内嵌）
- 启动 token + Cookie 鉴权、路径穿越防护
- Git 可用性与仓库状态探测（降级路径）

未做（按 PRD 排期）：

- 前端编辑器（CodeMirror 6 Live Preview）——目前只有一个可勾选待办的极简首页
- 日程视图（本日 / 本周 / 本月）、标签树、全文搜索、文件列表分组 UI
- Git 自动 init / 防抖自动提交、文件监听
- 双链（P1，默认关闭）

## 运行

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
