# 数据约定

## 绝不改写已有笔记

PRD 红线：读取只解析，写只写新建文件或用户明确修改的那一行。任何批量格式化、自动补 frontmatter、统一换行，都不许做。

## 模板

- 用户模板：`<root>/templates/*.md`，新建弹窗里会列出来
- 内置模板：`docs/templates/daily-note.md`、`note.md`，用 `include_str!` 内嵌（**不要**改成运行时读文件）
- 占位符：`{{TITLE}}` `{{DATE}}` `{{DATETIME}}` `{{TAGS}}`（`render_template` / `render_template_full`）
- 日记模板骨架（v0.6 起）：

```markdown
---
title: {{DATE}}
type: diary
created: {{DATETIME}}
tags: []
---

# {{DATE}}

## 日程

## 待办

## 记录
```

## frontmatter

- 字段固定四个：`title` / `type` / `created` / `tags`
- 渲染时由 `splitFrontmatter()` 剥离，渲染成「元数据卡片」；**正文里不能再出现 `---` 分隔线**
- 编辑视图显示原文（允许用户改）

## 待办优先级

- 语法 `!p0`–`!p9`，大小写不敏感，前后必须是空白或行首/行尾
- `!p10`、`a!p1`、`!p1!p2` 都不识别（`markdown.rs::extract_priority` 有单测）
- 展示时从 `text` 去掉标记、用 `priority` 字段单独渲染徽标；**`raw_line` 保留原始文本**

## 待办回写与 `raw_line`

`raw_line` = 去掉行尾完成标记后的整行（含缩进与 `- [ ]`）。三级定位完全依赖它逐字匹配，**改解析逻辑时不要动它的生成方式**，否则回写会定位失败。

## 完成标记

- 勾选：`- [ ]` → `- [x]`，行尾追加 `✅ YYYY-MM-DD HH:mm`
- 取消：整段 `✅ …` 移除
- 搜索/索引前剥离 `✅` 起到行尾；未完成行里的 `✅` 不剥离

## 编码与行尾

- 读取剥 UTF-8 BOM（Windows 编辑器常见），否则首个标题/行首待办判定全失配
- 响应带 `eol`（lf/crlf）；保存与行回写按原 `eol` 还原，避免整篇 diff
- 非 UTF-8（疑似 GBK）：只记 warning、跳过，不写

## 删除

删除走 `DELETE /api/files/*path`，文件移到 `<root>/.trash/时间戳-文件名`，索引移除、进 Git 队列。**不做硬删**。
