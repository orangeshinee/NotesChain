//! 行级 Markdown 解析：代码块围栏、待办、标签、完成标记剥离。
//!
//! 这里刻意不引 pulldown-cmark：全量 AST 对「逐行找待办」是浪费，
//! 一个围栏状态机 + 逐行匹配就够，且对 20 MB 语料是线性零分配的。

/// 单条待办。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
    /// 行号，从 1 开始
    pub line: usize,
    /// 行首缩进宽度（空格数），用于首页保留层级
    pub indent: usize,
    /// 去掉行尾完成标记后的整行文本，即 PRD 3.2.3 的「原始文本」
    pub raw_line: String,
    /// 待办正文（去掉 `- [ ] ` 前缀与完成标记）
    pub text: String,
    /// 行内标签（含正文与待办行上的）
    pub tags: Vec<String>,
    pub done: bool,
    /// 完成时间原文，如 `2026-11-01 09:12`
    pub done_at: Option<String>,
}

/// 标记每一行是否处于代码块内（含围栏行本身）。
pub fn code_block_flags(lines: &[&str]) -> Vec<bool> {
    let mut flags = Vec::with_capacity(lines.len());
    // (围栏字符, 围栏长度)
    let mut fence: Option<(char, usize)> = None;

    for line in lines {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let mut marked = false;

        if indent <= 3 {
            match fence {
                Some((ch, len)) => {
                    let run = leading_run(trimmed, ch);
                    if run >= len && trimmed[run..].trim().is_empty() {
                        fence = None;
                        marked = true; // 收尾围栏行本身也算代码块
                    }
                }
                None => {
                    for ch in ['`', '~'] {
                        let run = leading_run(trimmed, ch);
                        if run >= 3 {
                            fence = Some((ch, run));
                            marked = true;
                            break;
                        }
                    }
                }
            }
        }

        flags.push(marked || fence.is_some());
    }

    flags
}

fn leading_run(s: &str, ch: char) -> usize {
    s.chars().take_while(|c| *c == ch).count()
}

/// 是否为待办行（行首允许缩进，`- [ ]` 或 `- [x]`）。
pub fn is_todo_line(line: &str) -> bool {
    match todo_prefix(line) {
        Some((_, _, _)) => true,
        None => false,
    }
}

/// 拆解待办行前缀，返回 (缩进宽度, 是否已完成, 正文起始字节偏移)。
fn todo_prefix(line: &str) -> Option<(usize, bool, usize)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    let indent = i;
    // 至少 `- [ ]`
    if bytes.len() < i + 5 {
        return None;
    }
    if bytes[i] != b'-' || bytes[i + 1] != b' ' || bytes[i + 2] != b'[' {
        return None;
    }
    let state = bytes[i + 3];
    if bytes[i + 4] != b']' {
        return None;
    }
    let done = match state {
        b' ' => false,
        b'x' | b'X' => true,
        _ => return None,
    };
    let mut start = i + 5;
    if start < bytes.len() && bytes[start] == b' ' {
        start += 1;
    }
    Some((indent, done, start))
}

/// 剥离行尾完成标记：以 `✅` 为分隔符，从该字符起到行尾整段去掉。
/// 只对已完成的待办行生效（PRD 3.2.2）。
pub fn strip_done_marker(line: &str) -> String {
    match todo_prefix(line) {
        Some((_, true, _)) => match line.find('✅') {
            Some(idx) => line[..idx].trim_end().to_string(),
            None => line.trim_end().to_string(),
        },
        _ => line.trim_end().to_string(),
    }
}

/// 提取行尾完成时间，返回 `✅` 之后的原文（去空白）。
pub fn done_at(line: &str) -> Option<String> {
    let (_, done, _) = todo_prefix(line)?;
    if !done {
        return None;
    }
    let idx = line.find('✅')?;
    let rest = line[idx + '✅'.len_utf8()..].trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// 解析全部待办。代码块内的 `- [ ]` 不算待办。
pub fn parse_todos(content: &str) -> Vec<Todo> {
    let lines: Vec<&str> = content.lines().collect();
    let flags = code_block_flags(&lines);
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] {
            continue;
        }
        let Some((indent, done, start)) = todo_prefix(line) else {
            continue;
        };
        let raw_line = strip_done_marker(line);
        let mut text = line[start..].to_string();
        if done {
            if let Some(pos) = text.find('✅') {
                text.truncate(pos);
            }
        }
        let text = text.trim_end().to_string();
        out.push(Todo {
            line: idx + 1,
            indent,
            raw_line,
            tags: extract_tags(line),
            text,
            done,
            done_at: done_at(line),
        });
    }

    out
}

/// 提取标签。规则见 PRD 3.4：1-3 级、无空格、不以 `/` 开头或结尾、无 `//`。
/// 跳过代码块、行内代码、标题行首的 `#`。
pub fn extract_tags(line: &str) -> Vec<String> {
    if line.trim_start().starts_with('#') {
        return Vec::new();
    }
    let mut tags = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_code = false;

    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if c == '#' && !in_code {
            // `#` 前必须是行首或空白，避免 `a#b` 之类被当成标签
            let prev_ok = i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(';
            if !prev_ok {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let mut name = String::new();
            while j < chars.len() {
                let ch = chars[j];
                if ch.is_whitespace() {
                    break;
                }
                if is_tag_char(ch) {
                    name.push(ch);
                    j += 1;
                } else {
                    break;
                }
            }
            if valid_tag(&name) && !tags.contains(&name) {
                tags.push(name);
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }

    tags
}

fn is_tag_char(c: char) -> bool {
    c == '/' || c == '-' || c == '_' || c.is_alphanumeric() || is_cjk(c)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

fn valid_tag(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return false;
    }
    if name.split('/').count() > 3 {
        return false;
    }
    name.split('/').all(|seg| !seg.is_empty())
}

/// 待办行的完成状态；非待办行返回 None。
pub fn todo_state(line: &str) -> Option<bool> {
    todo_prefix(line).map(|(_, done, _)| done)
}

/// 待办的显示文本：去掉 `- [ ]` 前缀与行尾完成标记。
pub fn todo_text(line: &str) -> String {
    let stripped = strip_done_marker(line);
    match todo_prefix(&stripped) {
        Some((_, _, start)) => stripped[start..].trim().to_string(),
        None => stripped.trim().to_string(),
    }
}

/// 收集文件里出现的全部标签（去重）。代码块与标题行里的 `#` 不算。
pub fn collect_tags(content: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let flags = code_block_flags(&lines);
    let mut out: Vec<String> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] {
            continue;
        }
        for tag in extract_tags(line) {
            if !out.contains(&tag) {
                out.push(tag);
            }
        }
    }
    out
}

/// 历史遗留的 `todo:` 写法（PRD 3.2.1）：只识别、不改写，供一次性迁移清单使用。
/// 命中形如 `todo: xxx`、`- todo: xxx`、`* TODO：xxx`。
pub fn find_legacy_todo(content: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let flags = code_block_flags(&lines);
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] {
            continue;
        }
        let mut rest = line.trim_start();
        // 去掉列表符号
        for bullet in ["- ", "* ", "+ "] {
            if let Some(stripped) = rest.strip_prefix(bullet) {
                rest = stripped.trim_start();
                break;
            }
        }
        let lower = rest.to_lowercase();
        if lower.starts_with("todo:") || lower.starts_with("todo：") {
            out.push((idx + 1, line.trim().to_string()));
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_legacy_todo_lines() {
        let doc = "\
- [ ] 正常待办
todo: 没有列表符号的老写法
- todo: 带列表符号
* TODO：大写与全角冒号

```
todo: 代码块里的不算
```
";
        let hits = find_legacy_todo(doc);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].0, 2);
        assert!(hits[2].1.contains("TODO"));
    }
    #[test]
    fn fence_flags() {
        let content = "- [ ] a\n```\n- [ ] b\n```\n- [ ] c\n";
        let lines: Vec<&str> = content.lines().collect();
        let flags = code_block_flags(&lines);
        assert_eq!(flags, vec![false, true, true, true, false]);
    }

    #[test]
    fn fence_tilde_and_unclosed() {
        let content = "~~~sql\nselect 1;\n- [ ] x";
        let lines: Vec<&str> = content.lines().collect();
        let flags = code_block_flags(&lines);
        assert_eq!(flags, vec![true, true, true]);
    }

    #[test]
    fn todo_parse_and_done_marker() {
        let content = "\
# 2026-11-01

## 待办

- [ ] 发供应链周报 #工作/供应链
- [x] 回复供应商邮件 #工作/采购 ✅ 2026-11-01 09:12
  - [ ] 子项
";
        let todos = parse_todos(content);
        assert_eq!(todos.len(), 3);
        assert_eq!(todos[0].line, 5);
        assert_eq!(todos[0].text, "发供应链周报 #工作/供应链");
        assert_eq!(todos[0].tags, vec!["工作/供应链"]);
        assert!(!todos[0].done);
        assert!(todos[1].done);
        assert_eq!(todos[1].done_at.as_deref(), Some("2026-11-01 09:12"));
        assert_eq!(todos[1].raw_line, "- [x] 回复供应商邮件 #工作/采购");
        assert_eq!(todos[2].indent, 2);
    }

    #[test]
    fn strip_marker_only_for_done() {
        // 未完成行里的 ✅ 不该被吞掉
        let line = "- [ ] 检查 ✅ 这个符号";
        assert_eq!(strip_done_marker(line), "- [ ] 检查 ✅ 这个符号");
        let done = "- [x] 检查 ✅ 2026-11-01 09:12";
        assert_eq!(strip_done_marker(done), "- [x] 检查");
    }

    #[test]
    fn tags_rules() {
        assert_eq!(extract_tags("写点 #工作/供应链/KPI 的东西"), vec!["工作/供应链/KPI"]);
        assert!(extract_tags("这行没有标签").is_empty());
        assert!(extract_tags("## 标题里的 #不算").is_empty());
        assert_eq!(extract_tags("行内代码 `#不算` 后面 #算"), vec!["算"]);
        // 四级标签不合法
        assert!(extract_tags("#a/b/c/d").is_empty());
        // 首尾斜杠不合法
        assert!(extract_tags("#/abc").is_empty());
        assert!(extract_tags("#abc/").is_empty());
    }
}
