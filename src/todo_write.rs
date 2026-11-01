//! 待办回写定位（PRD 3.2.3 的三级解析）与原子写入。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::markdown;

#[derive(Debug, PartialEq, Eq)]
pub enum LocateError {
    /// 零匹配：该待办已不存在或内容已变更
    NotFound,
    /// 多处匹配且无法用行号就近区分
    Ambiguous,
}

impl LocateError {
    pub fn code(&self) -> &'static str {
        "todo_relocate_failed"
    }

    pub fn message(&self) -> &'static str {
        match self {
            LocateError::NotFound => "该待办已不存在或内容已变更，请重新加载",
            LocateError::Ambiguous => "该待办在文件中有多处相同文本且无法确定位置，请重新加载",
        }
    }
}

/// 三级解析：行号快路径 → 全文唯一匹配 → 行号就近 → 拒绝。
///
/// `submitted_line` 为 1 起始行号；`raw_line` 为去掉完成标记后的整行文本。
pub fn locate(
    content: &str,
    submitted_line: usize,
    raw_line: &str,
) -> Result<usize, LocateError> {
    let lines: Vec<&str> = content.lines().collect();
    let flags = markdown::code_block_flags(&lines);
    let target = raw_line.trim_end();

    let mut hits: Vec<usize> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] || !markdown::is_todo_line(line) {
            continue;
        }
        if markdown::strip_done_marker(line).trim_end() == target {
            hits.push(idx);
        }
    }

    // 一级：行号命中
    if submitted_line >= 1 {
        let fast = submitted_line - 1;
        if hits.contains(&fast) {
            return Ok(fast);
        }
    }
    match hits.len() {
        0 => Err(LocateError::NotFound),
        1 => Ok(hits[0]),
        _ => {
            // 三级：取与提交行号距离最近的一处；并列则拒绝
            let mut best: Option<(usize, usize)> = None;
            let mut tie = false;
            for &idx in &hits {
                let distance = (idx + 1).abs_diff(submitted_line);
                match best {
                    None => best = Some((idx, distance)),
                    Some((_, d)) if distance < d => {
                        best = Some((idx, distance));
                        tie = false;
                    }
                    Some((_, d)) if distance == d => tie = true,
                    _ => {}
                }
            }
            match best {
                Some((idx, _)) if !tie => Ok(idx),
                _ => Err(LocateError::Ambiguous),
            }
        }
    }
}

/// 把某行改写为勾选 / 取消勾选后的样子（保留缩进）。
pub fn toggle_line(line: &str, done: bool, now: &str) -> String {
    let stripped = markdown::strip_done_marker(line);
    let indent_len = stripped.len() - stripped.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = stripped.split_at(indent_len);
    let body = strip_body(rest);
    if done {
        format!("{indent}- [x] {body} ✅ {now}")
    } else {
        format!("{indent}- [ ] {body}")
    }
}

/// 去掉 `- [ ] ` / `- [x] ` 前缀（不含缩进处理）。
fn strip_body(line: &str) -> String {
    let body = line
        .strip_prefix("- [ ] ")
        .or_else(|| line.strip_prefix("- [x] "))
        .or_else(|| line.strip_prefix("- [X] "))
        .or_else(|| line.strip_prefix("- [ ]"))
        .or_else(|| line.strip_prefix("- [x]"))
        .or_else(|| line.strip_prefix("- [X]"))
        .unwrap_or(line);
    body.trim_end().to_string()
}

/// 原子写入：同目录临时文件 + 改名（PRD 5.3 第 2 条）。
pub fn atomic_write(path: &Path, content: &str) -> anyhow::Result<()> {
    let tmp = temp_path(path);
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("创建临时文件失败: {}", tmp.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("替换目标文件失败: {}", path.display()))?;
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "note.md".to_string());
    path.with_file_name(format!(".noteschain.{name}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
# 2026-11-01

## 待办

- [ ] 发周报 #工作
- [ ] 先对采购模块
- [ ] 先对采购模块
- [x] 已完成的事 ✅ 2026-11-01 09:12
";

    #[test]
    fn level1_line_hit() {
        assert_eq!(locate(DOC, 5, "- [ ] 发周报 #工作"), Ok(4));
    }

    #[test]
    fn level2_after_external_insert() {
        // 模拟外部在文件上方插了两行，原第 5 行变成第 7 行
        let shifted = "# 2026-11-01\n\n外部插入\n- [ ] 发周报 #工作\n- [ ] 先对采购模块\n- [ ] 先对采购模块\n- [x] 已完成的事 ✅ 2026-11-01 09:12\n".to_string();
        let idx = locate(&shifted, 5, "- [ ] 发周报 #工作").unwrap();
        assert_eq!(idx, 3);
    }

    #[test]
    fn level3_nearest_wins() {
        // 第 6、7 行文本完全相同：提交行号 5 应命中第 6 行（索引 5）
        assert_eq!(locate(DOC, 5, "- [ ] 先对采购模块"), Ok(5));
        // 提交行号 7 应命中第 7 行（索引 6）
        assert_eq!(locate(DOC, 7, "- [ ] 先对采购模块"), Ok(6));
        // 提交行号 6 直接命中第 6 行（一级快路径）
        assert_eq!(locate(DOC, 6, "- [ ] 先对采购模块"), Ok(5));
    }

    #[test]
    fn rejects_when_ambiguous_or_missing() {
        let doc = "- [ ] 同一个\n\n- [ ] 同一个\n";
        // 行号 2 不在命中集合里，距离并列（|1-2|=1 与 |3-2|=1）
        assert_eq!(locate(doc, 2, "- [ ] 同一个"), Err(LocateError::Ambiguous));
        assert_eq!(locate(DOC, 5, "- [ ] 不存在的事"), Err(LocateError::NotFound));
    }

    #[test]
    fn ignores_code_block_todos() {
        let doc = "- [ ] 真的\n\n```\n- [ ] 真的\n```\n";
        assert_eq!(locate(doc, 1, "- [ ] 真的"), Ok(0));
        // 只有代码块里那一处时不算匹配
        let only_code = "```\n- [ ] 真的\n```\n";
        assert_eq!(locate(only_code, 2, "- [ ] 真的"), Err(LocateError::NotFound));
    }

    #[test]
    fn toggle_roundtrip() {
        let line = "  - [ ] 子项 #标签";
        let done = toggle_line(line, true, "2026-11-01 09:12");
        assert_eq!(done, "  - [x] 子项 #标签 ✅ 2026-11-01 09:12");
        let back = toggle_line(&done, false, "2026-11-01 09:12");
        assert_eq!(back, "  - [ ] 子项 #标签");
    }

    #[test]
    fn toggle_already_done_is_idempotent() {
        let done = "- [x] 一件事 ✅ 2026-11-01 09:12";
        assert_eq!(toggle_line(done, true, "2026-11-01 10:00"), "- [x] 一件事 ✅ 2026-11-01 10:00");
    }
}
