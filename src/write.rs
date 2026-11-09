//! 定向写入的纯函数层：按标题把一行追加到 Markdown 文件的指定段。
//!
//! 从 `api.rs` 抽出，HTTP 接口与 CLI 共用。红线（AGENTS.md）：绝不改写已有笔记，
//! 只允许「追加一行」或「新建文件写模板」。实现上按行保留原始 eol 与 BOM，
//! 不做整篇规范化，原有行的字节除必要的行尾分隔外一字不改。

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

}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
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

/// 一行原文 + 它自己的行尾。逐行保留，避免混合 eol 被整篇统一。
#[derive(Debug, Clone)]
struct Line {
    text: String,
    eol: &'static str,
}

fn split_lines(raw: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (idx, byte) in raw.bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let seg = &raw[start..idx];
        let (text, eol) = match seg.strip_suffix('\r') {
            Some(t) => (t, "\r\n"),
            None => (seg, "\n"),
        };
        out.push(Line {
            text: text.to_string(),
            eol,
        });
        start = idx + 1;
    }
    if start < raw.len() {
        out.push(Line {
            text: raw[start..].to_string(),
            eol: "",
        });
    }
    out
}

fn join_lines(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&line.text);
        out.push_str(line.eol);
    }
    out
}

/// 新增行沿用文件末尾的行尾风格；空文件用 LF。
/// 不能用「第一个」行尾：混合行尾文件里那会把新段写成与尾部不一致的风格。
fn preferred_eol(lines: &[Line]) -> &'static str {
    lines
        .iter()
        .rev()
        .find(|l| !l.eol.is_empty())
        .map(|l| l.eol)
        .unwrap_or("\n")
}

/// 把一行插入目标段，返回写回磁盘的完整文本（原有行的 eol / BOM / 尾空格保留）。
pub fn insert_line(
    raw: &str,
    sections: &[String],
    line: &str,
    unique: bool,
) -> Result<InsertOutcome, WriteError> {
    if sections.is_empty() {
        return Ok(append_at_end(raw, line, unique));
    }
    let section = resolve_section(raw, sections)?;
    let target_level = section.chars().take_while(|c| *c == '#').count().max(1);
    let mut lines = split_lines(raw);
    let refs: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let flags = markdown::code_block_flags(&refs);

    let heading = lines
        .iter()
        .enumerate()
        .find(|(idx, l)| !flags[*idx] && strip_bom(&l.text).trim() == section)
        .map(|(idx, _)| idx);

    let Some(heading_idx) = heading else {
        // 段不存在：在末尾追加该标题与内容。原文一字不改，只给末行补必要的换行。
        let eol = preferred_eol(&lines);
        if let Some(last) = lines.last_mut() {
            if last.eol.is_empty() {
                last.eol = eol;
            }
            lines.push(Line {
                text: String::new(),
                eol,
            });
        }
        lines.push(Line {
            text: section.clone(),
            eol,
        });
        lines.push(Line {
            text: String::new(),
            eol,
        });
        let line_no = lines.len() + 1;
        lines.push(Line {
            text: line.to_string(),
            eol,
        });
        return Ok(InsertOutcome {
            content: join_lines(&lines),
            line: line_no,
            section,
            skipped: false,
        });
    };

    let start = heading_idx + 1;
    let end = section_end(&lines, &flags, start, target_level);

    if unique {
        if let Some(pos) = (start..end)
            .find(|&idx| !flags[idx] && lines[idx].text.trim() == line.trim())
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
    while insert_at > start && lines[insert_at - 1].text.trim().is_empty() {
        insert_at -= 1;
    }
    let eol = if insert_at > 0 && !lines[insert_at - 1].eol.is_empty() {
        lines[insert_at - 1].eol
    } else {
        preferred_eol(&lines)
    };
    // 插在末尾时，原末行若没有换行，补一个分隔符，保证新行独立成行。
    if insert_at > 0 && lines[insert_at - 1].eol.is_empty() {
        lines[insert_at - 1].eol = eol;
    }
    lines.insert(
        insert_at,
        Line {
            text: line.to_string(),
            eol,
        },
    );
    Ok(InsertOutcome {
        content: join_lines(&lines),
        line: insert_at + 1,
        section,
        skipped: false,
    })
}

/// 文件尾追加一行（notes/append 的旧行为）。保留原 eol 与 BOM，只补必要的行尾分隔。
pub fn append_at_end(raw: &str, line: &str, unique: bool) -> InsertOutcome {
    let mut lines = split_lines(raw);
    let refs: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let flags = markdown::code_block_flags(&refs);
    if unique {
        if let Some(pos) = (0..lines.len())
            .find(|&idx| !flags[idx] && lines[idx].text.trim() == line.trim())
        {
            return InsertOutcome {
                content: raw.to_string(),
                line: pos + 1,
                section: String::new(),
                skipped: true,
            };
        }
    }
    let eol = preferred_eol(&lines);
    if let Some(last) = lines.last_mut() {
        if last.eol.is_empty() {
            last.eol = eol;
        }
    }
    let line_no = lines.len() + 1;
    lines.push(Line { text: line.to_string(), eol });
    InsertOutcome {
        content: join_lines(&lines),
        line: line_no,
        section: String::new(),
        skipped: false,
    }
}

/// 候选标题里取第一个真实存在的；都不存在则用第一个新建段。
/// 代码块内的同名标题不算命中。
fn resolve_section(raw: &str, sections: &[String]) -> Result<String, WriteError> {
    if sections.is_empty() {
        return Err(WriteError::SectionRequired);
    }
    let lines = split_lines(raw);
    let refs: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    let flags = markdown::code_block_flags(&refs);
    for candidate in sections {
        if lines
            .iter()
            .enumerate()
            .any(|(idx, l)| !flags[idx] && strip_bom(&l.text).trim() == candidate.as_str())
        {
            return Ok(candidate.clone());
        }
    }
    Ok(sections[0].clone())
}

/// 段尾：下一个层级不高于目标标题的标题之前。代码块内的标题不算段尾。
fn section_end(lines: &[Line], flags: &[bool], from: usize, level: usize) -> usize {
    for idx in from..lines.len() {
        if flags[idx] {
            continue;
        }
        let trimmed = strip_bom(&lines[idx].text).trim_start();
        if trimmed.starts_with('#') {
            let found = trimmed.chars().take_while(|c| *c == '#').count();
            let rest = &trimmed[found..];
            // CommonMark ATX 标题：1-6 个 #，后接空格 / tab 或行尾。
            // `#标签`、`##不是标题` 只是普通文本，不能提前结束当前段。
            let is_atx = (1..=6).contains(&found)
                && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'));
            if is_atx && found <= level {
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
        assert!(!out.content.contains('\r'));
        assert_eq!(out.line, 6);
    }

    #[test]
    fn keeps_crlf() {
        let crlf = DOC.replace('\n', "\r\n");
        let out = insert(&crlf, "## 待办", "- [ ] 新的一项");
        assert!(out.content.contains("\r\n- [ ] 新的一项\r\n"));
        assert_eq!(out.content.replace("\r\n", "").matches('\r').count(), 0);
    }

    #[test]
    fn mixed_eol_is_preserved_line_by_line() {
        let mixed = "A\r\nB\nC\n";
        let out = insert(mixed, "## 记录", "NEW");
        assert_eq!(out.content.matches("\r\n").count(), 1, "只原有的一行是 CRLF");
        assert!(out.content.contains("B\n"), "LF 行不能变成 CRLF");
        assert!(!out.content.contains("B\r\n"));
        assert!(out.content.contains("NEW"));
    }

    #[test]
    fn no_trailing_newline_gets_only_a_separator() {
        let out = insert("# 标题\n\n## 待办\n\n- [ ] 旧", "## 待办", "- [ ] 新");
        assert!(out.content.contains("- [ ] 旧\n- [ ] 新"));
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
        assert!(out.content.ends_with("## 待办\n\n- [ ] 真的待办\n"));
    }

    #[test]
    fn code_block_heading_does_not_end_section() {
        let doc = "## 待办\n\n```md\n## 记录\n```\n\n- [ ] 仍在待办段\n";
        let out = insert(doc, "## 待办", "- [ ] 新的一项");
        assert!(out.content.contains("- [ ] 仍在待办段\n- [ ] 新的一项\n"));
    }

    #[test]
    fn h3_section_ends_at_next_h3() {
        let doc = "## 记录\n\n### 甲\n\n旧甲\n\n### 乙\n\n旧乙\n";
        let out = insert(doc, "### 甲", "新甲");
        assert!(
            out.content.find("新甲").unwrap() < out.content.find("### 乙").unwrap(),
            "H3 段不能越过下一个 H3：{}",
            out.content
        );
    }

    #[test]
    fn h3_does_not_end_h2_section() {
        let doc = "## 待办\n\n### 子小节\n\n- [ ] 子项\n";
        let out = insert(doc, "## 待办", "- [ ] 新的一项");
        assert!(out.content.contains("- [ ] 子项\n- [ ] 新的一项\n"));
    }

    #[test]
    fn creates_section_without_erasing_trailing_spaces() {
        let out = insert("正文   ", "## 会议", "一行记录");
        assert!(out.content.starts_with("正文   "), "{}", out.content);
        assert!(out.content.ends_with("## 会议\n\n一行记录\n"));
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
    fn empty_sections_appends_at_end() {
        let out = insert_line(DOC, &[], "一行", false).unwrap();
        assert!(out.content.ends_with("一行\n"));
        assert_eq!(out.section, "");
    }

    #[test]
    fn hash_without_space_does_not_end_section() {
        let doc = "## 待办\n\n- [ ] 旧的一项\n\n#工作\n普通文本\n\n## 记录\n";
        let out = insert(doc, "## 待办", "- [ ] 新的一项");
        assert_eq!(
            out.content,
            "## 待办\n\n- [ ] 旧的一项\n\n#工作\n普通文本\n- [ ] 新的一项\n\n## 记录\n"
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
