# 工作流与风格

## 启动

```powershell
.\target\release\noteschain.exe --dir "D:\MyNotes" --port 8090
```

| 参数 | 说明 |
| --- | --- |
| `--dir` | 项目根（必填） |
| `--port` | 默认 **8090**，只绑 127.0.0.1 |
| `--links` | 启用双链 `[[目标]]`（默认关闭） |
| `--no-git` | 关闭 Git 自动提交，其余功能照常 |
| `--no-token` | 禁用 token 鉴权（仅建议个人独占机器；默认仍校验） |
| `--commit-debounce` | Git 自动提交防抖秒数，默认 **300**（停止变更 5 分钟合并提交） |
| `--commit-force` | 连续编辑强制提交秒数，默认 **1800** |

启动会打印带 token 的地址；token 只存在进程内存，**重启即失效**。

> 端口提示：本机 8080 常被 Steam 的远程调试端口占用，所以默认改成了 8090。

## 提交

- 一个改动一个提交；提交信息注明对应 issue 编号（`#16`）或 PRD 章节
- 修复类提交写**根因**，不写「修复 bug」
- 提交前 `git status`：只应有源码/文档。`.demo/`、`target/`、`web/node_modules` 已忽略
- 不要提交 `.demo/` 下的测试语料

## 分支保护与发布

- master 规则集配置在 `.github/rulesets/master.json`，要求 PR、合并提交和双平台 CI；禁止删除与强推。
- 应用规则集（需仓库管理员权限）：
  `gh api --method POST repos/orangeshinee/NotesChain/rulesets --input .github/rulesets/master.json`
- 发布顺序：版本 bump 与代码同一个 PR → CI 通过并合并 → master 提交上打 annotated tag → Release 自动执行。
- Release 只接受 master 上的 tag；不要移动已发布 tag。

## 代码风格

**Rust**
- 错误：接口层用 `ApiError`（固定错误码枚举，不要随意新增 `code` 字符串）；内部用 `anyhow`
- 新增接口：先想清楚要不要 `resolve()`、要不要按文件锁、要不要 `notify + revision`
- 注释用中文，解释「为什么」

**前端**
- 无框架、不引第三方运行时库；建 DOM 用 `el(tag, cls, text)`
- 颜色/间距走 `:root` 变量；暗色用 `prefers-color-scheme`
- 交互类改动先在浏览器里点一遍，再用 `browser-probe` 固化

## 文档

| 场景 | 改哪个 |
| --- | --- |
| 产品行为变化 | `docs/noteschain-prd-v05.md`（在 v0.6 段追加修订说明，不要直接改历史章节） |
| 新增/修复反馈 | `docs/issues-v0.2.md` 追加一行，标 ✅ |
| 架构/约定变化 | `docs/agents/*.md` 对应文件 |
| 状态变化 | `docs/handover.md`（只记状态、未完成、性能） |
