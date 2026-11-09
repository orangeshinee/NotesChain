//! 定向写入的纯函数层：按标题把一行追加到 Markdown 文件的指定段。
//!
//! 从 `api.rs` 抽出，HTTP 接口与 CLI 共用。红线（AGENTS.md）：绝不改写已有笔记，
//! 只允许「追加一行」或「新建文件写模板」。因此这里保留原 eol 与 BOM，不做整篇规范化。

use crate::markdown;

pub const TODO_SECTION: &str = "## 待办";
pub const SCHEDULE_SECTIONS: &[&str] = &["## 日程", "## 今日日程"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Todo,
    Schedule,
    Line,
}

impl Kind {
    /// 缺省标题。日程用候选列表，兼容模板改名前的老笔记（`## 今日日程`）。
    pub fn default_sections(self) -> Vec<String> {
        match self {
            Kind::Todo => vec![TODO_SECTION.to_string()],
            Kind::Schedule => SCHEDULE_SECTIONS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            Kind::Line => Vec::new(),
        }
    }

    /// 行格式化，行为与旧 append_todo / append_schedule 保持一致。
    pub fn format_line(self, text: &str) -> String {
        match self {
            Kind::Todo => {
                let body = text
                    .trim_start_matches("- [ ]")
                    .trim_start_matches("- [x]")
                    .trim_start_matches("- [X]")
                    .trim();
                format!("- [ ] {body}")
            }
            Kind::Schedule => {
                if text.starts_with('-') {
                    text.to_string()
                } else {
                    format!("- {text}")
                }
            }
            Kind::Line => text.to_string(),
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Kind::Todo => "/api/todos/append",
            Kind::Schedule => "/api/schedule/append",
            Kind::Line => "/api/lines/append",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    SectionRequired,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::SectionRequired => write!(f, "缺少标题（--section）"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InsertOutcome {
    pub content: String,
    /// 1 起始行号
    pub line: usize,
    pub section: String,
    /// `--unique` 命中已有行，未写入
    pub skipped: bool,
}

/// 把一行插入目标段，返回写回磁盘的完整文本（原 eol / BOM 保留）。
pub fn insert_line(
    raw: &str,
    sections: &[String],
    line: &str,
    unique: bool,
) -> Result<InsertOutcome, WriteError> {
    let section = resolve_section(raw, sections)?;
    // `str::lines()` 会剥掉 CRLF 的 `\r`，join 时按检测出的原 eol 还原，避免整篇改写
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let trailing_newline = raw.ends_with('\n');
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let flags = markdown::code_block_flags(&refs);

    let heading = lines
        .iter()
        .enumerate()
        .find(|(idx, text)| !flags[*idx] && strip_bom(text).trim() == section)
        .map(|(idx, _)| idx);

    let Some(heading_idx) = heading else {
        // 段不存在：在文件末尾追加该标题与内容，沿用原 eol
        let mut text = raw.trim_end().to_string();
        if !text.is_empty() {
            text.push_str(eol);
            text.push_str(eol);
        }
        text.push_str(&section);
        text.push_str(eol);
        text.push_str(eol);
        text.push_str(line);
        text.push_str(eol);
        let line_no = text.lines().count();
        return Ok(InsertOutcome {
            content: text,
            line: line_no,
            section,
            skipped: false,
        });
    };

    let start = heading_idx + 1;
    let end = section_end(&lines, &flags, start);

    if unique {
        if let Some(pos) = (start..end)
            .find(|&idx| !flags[idx] && lines[idx].trim() == line.trim())
        {
            return Ok(InsertOutcome {
                content: raw.to_string(),
                line: pos + 1,
                section,
                skipped: true,
            });
        }
    }

    let mut insert_at = end;
    while insert_at > start && lines[insert_at - 1].trim().is_empty() {
        insert_at -= 1;
    }
    lines.insert(insert_at, line.to_string());
    let mut content = lines.join(eol);
    if trailing_newline || !content.is_empty() {
        content.push_str(eol);
    }
    Ok(InsertOutcome {
        content,
        line: insert_at + 1,
        section,
        skipped: false,
    })
}

/// 候选标题里取第一个真实存在的；都不存在则用第一个新建段。
/// 代码块内的同名标题不算命中。
fn resolve_section(raw: &str, sections: &[String]) -> Result<String, WriteError> {
    if sections.is_empty() {
        return Err(WriteError::SectionRequired);
    }
    let lines: Vec<&str> = raw.lines().collect();
    let flags = markdown::code_block_flags(&lines);
    for candidate in sections {
        if lines
            .iter()
            .enumerate()
            .any(|(idx, text)| !flags[idx] && strip_bom(text).trim() == candidate.as_str())
        {
            return Ok(candidate.clone());
        }
    }
    Ok(sections[0].clone())
}

/// 段尾：下一个一级 / 二级标题之前。代码块内的标题不算段尾。
fn section_end(lines: &[String], flags: &[bool], from: usize) -> usize {
    for idx in from..lines.len() {
        if flags[idx] {
            continue;
        }
        let trimmed = strip_bom(&lines[idx]).trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level <= 2 {
                return idx;
            }
        }
    }
    lines.len()
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# 2026-11-09\n\n## 待办\n\n- [ ] 旧的一项\n\n## 记录\n\n正文\n";

    fn insert(raw: &str, section: &str, line: &str) -> InsertOutcome {
        insert_line(raw, &[section.to_string()], line, false).unwrap()
    }

    #[test]
    fn keeps_lf() {
        let out = insert(DOC, "## 待办", "- [ ] 新的一项");
        assert!(out.content.contains("\n- [ ] 新的一项\n"));
        assert!(!out.content.contains("\r\n"));
        assert_eq!(out.line, 6);
    }

    #[test]
    fn keeps_crlf() {
        let crlf = DOC.replace('\n', "\r\n");
        let out = insert(&crlf, "## 待办", "- [ ] 新的一项");
        assert!(out.content.contains("\r\n- [ ] 新的一项\r\n"));
        assert!(!out.content.replace("\r\n", "").contains('\r'));
        assert!(!out.content.contains("\n- [ ] 新的一项\n"));
    }

    #[test]
    fn keeps_bom_and_still_matches_heading() {
        let bom = format!("\u{feff}{DOC}");
        let out = insert(&bom, "## 待办", "- [ ] 新的一项");
        assert!(out.content.starts_with('\u{feff}'), "BOM 不能被改写");
        assert!(out.content.contains("- [ ] 新的一项"));
    }

    #[test]
    fn ignores_heading_inside_code_block() {
        let doc = "# 标题\n\n```md\n## 待办\n```\n\n## 记录\n\n正文\n";
        let out = insert(doc, "## 待办", "- [ ] 真的待办");
        // 代码块里的 `## 待办` 不算命中，应该在末尾新建真段
        assert!(out.content.ends_with("## 待办\n\n- [ ] 真的待办\n"));
    }

    #[test]
    fn code_block_heading_does_not_end_section() {
        let doc = "## 待办\n\n```md\n## 记录\n```\n\n- [ ] 仍在待办段\n";
        let out = insert(doc, "## 待办", "- [ ] 新的一项");
        // 代码块里的 `## 记录` 不能截断段尾
        assert!(out.content.contains("- [ ] 仍在待办段\n- [ ] 新的一项\n"));
    }

    #[test]
    fn h3_does_not_end_section() {
        let doc = "## 待办\n\n### 子小节\n\n- [ ] 子项\n";
        let out = insert(doc, "## 待办", "- [ ] 新的一项");
        assert!(out.content.contains("- [ ] 子项\n- [ ] 新的一项\n"));
    }

    #[test]
    fn creates_section_when_missing() {
        let out = insert("# 标题\n\n正文\n", "## 会议", "一行记录");
        assert!(out.content.ends_with("## 会议\n\n一行记录\n"));
        assert_eq!(out.section, "## 会议");
    }

    #[test]
    fn unique_skips_duplicate() {
        let out = insert_line(DOC, &["## 待办".to_string()], "- [ ] 旧的一项", true).unwrap();
        assert!(out.skipped);
        assert_eq!(out.line, 5);
        assert_eq!(out.content, DOC);
    }

    #[test]
    fn schedule_alias_prefers_existing_heading() {
        let doc = "# 今日\n\n## 今日日程\n\n- 09:00 晨会\n";
        let sections = Kind::Schedule.default_sections();
        let out = insert_line(doc, &sections, "- 10:00 周会", false).unwrap();
        assert_eq!(out.section, "## 今日日程");
        assert!(out.content.contains("- 09:00 晨会\n- 10:00 周会\n"));
        assert!(!out.content.contains("## 日程\n"));
    }

    #[test]
    fn line_requires_section() {
        assert_eq!(
            insert_line(DOC, &[], "一行", false).unwrap_err(),
            WriteError::SectionRequired
        );
    }

    #[test]
    fn format_line_matches_legacy_behaviour() {
        assert_eq!(Kind::Todo.format_line("- [ ] 买牛奶"), "- [ ] 买牛奶");
        assert_eq!(Kind::Todo.format_line("买牛奶"), "- [ ] 买牛奶");
        assert_eq!(Kind::Schedule.format_line("15:00 周会"), "- 15:00 周会");
        assert_eq!(Kind::Schedule.format_line("- 15:00 周会"), "- 15:00 周会");
        assert_eq!(Kind::Line.format_line("原文"), "原文");
    }
}
