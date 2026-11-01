//! 日程解析（PRD 3.3）：只认 `## 今日日程` 段里的 `HH:mm[-HH:mm] 名称`。
//!
//! 解析不了的行不丢，进「未识别」列表原样返回 —— 静默漏会是最坏的失败方式。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub start: String,
    pub end: Option<String>,
    pub title: String,
    pub tags: Vec<String>,
    pub place: Option<String>,
    pub line: usize,
}

#[derive(Debug, Default)]
pub struct Section {
    pub items: Vec<Item>,
    pub unrecognized: Vec<String>,
}

/// 提取一个日记文件里的日程段。
pub fn parse(content: &str) -> Section {
    let lines: Vec<&str> = content.lines().collect();
    let flags = crate::markdown::code_block_flags(&lines);
    let mut section = Section::default();
    let mut inside = false;

    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = trimmed.trim_start_matches('#').trim();
            if level == 2 && (title == "今日日程" || title == "日程") {
                inside = true;
                continue;
            }
            if inside && level <= 2 {
                inside = false;
            }
            continue;
        }
        if !inside || trimmed.is_empty() {
            continue;
        }
        match parse_line(trimmed, idx + 1) {
            Some(item) => section.items.push(item),
            None => section.unrecognized.push(trimmed.trim_start_matches("- ").trim().to_string()),
        }
    }

    section
        .items
        .sort_by(|a, b| a.start.cmp(&b.start).then(a.line.cmp(&b.line)));
    section
}

fn parse_line(line: &str, line_no: usize) -> Option<Item> {
    // 允许写成列表项或裸行
    let body = line.strip_prefix("- ").unwrap_or(line).trim();

    let (start, rest) = take_time(body)?;
    let (end, rest) = if let Some(after) = rest.strip_prefix('-') {
        let (end, rest) = take_time(after)?;
        (Some(end), rest)
    } else {
        (None, rest)
    };

    let mut title = rest.trim().to_string();
    if title.is_empty() {
        return None;
    }

    // `@地点`：取标题里第一个 @ 开头的词
    let mut place = None;
    if let Some(word) = title.split_whitespace().find(|w| w.starts_with('@')) {
        place = Some(word.trim_start_matches('@').to_string());
        title = title
            .split_whitespace()
            .filter(|w| *w != word)
            .collect::<Vec<_>>()
            .join(" ");
    }

    let tags = crate::markdown::extract_tags(body);
    // 标签已单独作为 chips 展示，标题里不再重复
    title = title
        .split_whitespace()
        .filter(|w| !w.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    // 去掉标签/地点后标题为空（如 `- 10:00 #工作`）→ 视为未识别，不产出空标题日程
    if title.trim().is_empty() {
        return None;
    }
    // 结束时间早于开始时间 → 视为未识别，不猜
    if let Some(end) = &end {
        if end <= &start {
            return None;
        }
    }

    Some(Item {
        start,
        end,
        title: title.trim().to_string(),
        tags,
        place,
        line: line_no,
    })
}

fn take_time(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    if bytes.len() < 5 {
        return None;
    }
    if !(bytes[0].is_ascii_digit() && bytes[1].is_ascii_digit() && bytes[2] == b':' && bytes[3].is_ascii_digit() && bytes[4].is_ascii_digit()) {
        return None;
    }
    let hour: u32 = s[0..2].parse().ok()?;
    let minute: u32 = s[3..5].parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((s[0..5].to_string(), &s[5..]))
}

/// 疑似日程：不在 `## 今日日程` 段内、但行首像时间的行（PRD 3.3 兜底 2）。
pub fn suspected_outside_section(content: &str) -> Vec<usize> {
    let lines: Vec<&str> = content.lines().collect();
    let flags = crate::markdown::code_block_flags(&lines);
    let mut inside = false;
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = trimmed.trim_start_matches('#').trim();
            if level == 2 && (title == "今日日程" || title == "日程") {
                inside = true;
                continue;
            }
            if inside && level <= 2 {
                inside = false;
            }
            continue;
        }
        if inside || trimmed.is_empty() {
            continue;
        }
        let body = trimmed.strip_prefix("- ").unwrap_or(trimmed).trim_start();
        if take_time(body).is_some() {
            out.push(idx + 1);
        }
    }

    out
}
#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
# 2026-11-01

## 今日日程

- 09:30-10:00 晨会 @线上
- 15:00-17:00 供应链周会 #工作/供应链
- 15:00 提交月报
- 记得带电脑充电器

## 待办

- [ ] 15:00 这条在待办段里，不算日程
";

    #[test]
    fn suspects_time_lines_outside_section() {
        let doc = "\
# 2026-11-01

## 今日日程

- 09:30-10:00 晨会

## 记录

- 15:00 这行像日程但不在日程段里
- 普通一行
";
        let hits = suspected_outside_section(doc);
        assert_eq!(hits, vec![9]);
    }
    #[test]
    fn parses_items_and_unknown() {
        let s = parse(DOC);
        assert_eq!(s.items.len(), 3);
        assert_eq!(s.items[0].start, "09:30");
        assert_eq!(s.items[0].end.as_deref(), Some("10:00"));
        assert_eq!(s.items[0].title, "晨会");
        assert_eq!(s.items[0].place.as_deref(), Some("线上"));
        assert_eq!(s.items[1].tags, vec!["工作/供应链"]);
        assert_eq!(s.items[2].start, "15:00");
        assert_eq!(s.items[2].end, None);
        assert_eq!(s.unrecognized, vec!["记得带电脑充电器"]);
    }

    #[test]
    fn ignores_todo_section_and_code_blocks() {
        let doc = "\
## 今日日程

- 10:00 开会

```text
- 11:00 代码块里的不算
```

## 待办

- [ ] 12:00 待办段里的不算
";
        let s = parse(doc);
        assert_eq!(s.items.len(), 1);
        assert!(s.unrecognized.is_empty());
    }

    #[test]
    fn reversed_range_is_unrecognized() {
        let s = parse("## 今日日程\n\n- 17:00-15:00 写反了\n");
        assert!(s.items.is_empty());
        assert_eq!(s.unrecognized.len(), 1);
    }

    #[test]
    fn sorts_by_start() {
        let s = parse("## 今日日程\n\n- 15:00 下午\n- 09:00 早上\n");
        assert_eq!(s.items[0].start, "09:00");
        assert_eq!(s.items[1].start, "15:00");
    }
}
