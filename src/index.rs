//! 内存索引：扫描笔记根目录、解析待办与标签、维护版本号。
//!
//! 不落盘（PRD 4.5）：索引与磁盘不一致时以重建为准。

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::markdown::{self, Todo};

#[derive(Debug, Clone)]
pub struct FileEntry {
    /// 相对 notes 根的路径，统一用 `/` 分隔
    pub path: String,
    pub mtime_ms: u64,
    pub size: u64,
    /// 版本号 = `mtime-内容哈希前16位`（PRD 5.3 第 1 条）
    pub version: String,
    /// 日记的日期（`YYYY-MM-DD`），非日记为 None
    pub diary_date: Option<String>,
    #[allow(dead_code)] // 全文搜索（PRD 4.5）将直接读索引里的正文，避免重复读盘
    pub content: String,
    pub todos: Vec<Todo>,
    /// 文件里出现的全部标签（去重，含待办行上的）
    pub tags: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Index {
    pub files: Vec<FileEntry>,
    /// 扫描时跳过的非 UTF-8 / 读取失败文件，供界面提示
    pub warnings: Vec<String>,
}

impl Index {
    pub fn scan(root: &Path) -> anyhow::Result<Index> {
        let ignore = Ignored::load(root);
        let mut index = Index::default();

        let walker = WalkDir::new(root).follow_links(false).sort_by_file_name();
        for entry in walker.into_iter().filter_entry(|e| !ignore.skip_entry(e.path())) {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    index.warnings.push(format!("遍历失败: {err}"));
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = relative_path(root, entry.path());
            if ignore.is_ignored(&rel) {
                continue;
            }
            if !rel.ends_with(".md") {
                continue;
            }
            match load_file(entry.path(), &rel) {
                Ok(file) => index.files.push(file),
                Err(err) => index.warnings.push(format!("{rel}: {err}")),
            }
        }

        index.files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(index)
    }


    /// `raw/` 下同名的裸名冲突（PRD 3.5.4）：返回 (裸名, 冲突文件路径列表)
    pub fn duplicate_raw_names(&self) -> Vec<(String, Vec<String>)> {
        use std::collections::BTreeMap;
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for file in &self.files {
            if !file.path.starts_with("raw/") || !file.path.ends_with(".md") {
                continue;
            }
            let stem = file
                .path
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_end_matches(".md")
                .to_string();
            groups.entry(stem).or_default().push(file.path.clone());
        }
        groups
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect()
    }
    pub fn upsert(&mut self, root: &Path, path: &str) -> anyhow::Result<()> {
        let abs = root.join(path);
        if !abs.exists() {
            self.files.retain(|f| f.path != path);
            return Ok(());
        }
        let file = load_file(&abs, path)?;
        match self.files.iter_mut().find(|f| f.path == path) {
            Some(slot) => *slot = file,
            None => self.files.push(file),
        }
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(())
    }

    /// 全部未完成待办，按 PRD 4.1 的排序规则：
    /// 日记按日期倒序在前，其余按路径字典序、行号升序在后。
    pub fn open_todos(&self) -> Vec<TodoRow> {
        let mut rows: Vec<TodoRow> = Vec::new();
        for file in &self.files {
            for todo in file.todos.iter().filter(|t| !t.done) {
                rows.push(TodoRow {
                    path: file.path.clone(),
                    date: file.diary_date.clone(),
                    line: todo.line,
                    indent: todo.indent,
                    text: todo.text.clone(),
                    raw_line: todo.raw_line.clone(),
                    tags: todo.tags.clone(),
                    version: file.version.clone(),
                });
            }
        }
        rows.sort_by(|a, b| match (&a.date, &b.date) {
            (Some(da), Some(db)) => db.cmp(da).then(a.path.cmp(&b.path)).then(a.line.cmp(&b.line)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.path.cmp(&b.path).then(a.line.cmp(&b.line)),
        });
        rows
    }
}

/// 首页待办行（序列化给前端）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TodoRow {
    pub path: String,
    pub date: Option<String>,
    pub line: usize,
    pub indent: usize,
    pub text: String,
    pub raw_line: String,
    pub tags: Vec<String>,
    pub version: String,
}

fn load_file(abs: &Path, rel: &str) -> anyhow::Result<FileEntry> {
    let meta = fs::metadata(abs).with_context(|| "读取文件元信息失败")?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let bytes = fs::read(abs).with_context(|| "读取文件失败")?;
    let content = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("不是合法的 UTF-8（疑似 GBK 老文件），已跳过，未做任何写入"))?;

    let diary_date = detect_diary(rel, &content);
    let todos = markdown::parse_todos(&content);
    let tags = markdown::collect_tags(&content);

    Ok(FileEntry {
        path: rel.to_string(),
        mtime_ms,
        size: meta.len(),
        version: version_of(mtime_ms, &content),
        diary_date,
        content,
        todos,
        tags,
    })
}

/// 版本号：mtime + 内容哈希（PRD 5.3 第 1 条）。
pub fn version_of(mtime_ms: u64, content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let hash = hex(&digest);
    format!("{mtime_ms}-{}", &hash[..16])
}


fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 文件类型判定（PRD 3.1.2）：路径为主，内容兜底。
pub fn detect_diary(rel: &str, content: &str) -> Option<String> {
    if let Some(date) = diary_from_path(rel) {
        return Some(date);
    }
    diary_from_heading(content)
}

fn diary_from_path(rel: &str) -> Option<String> {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let (year, month, name) = (parts[0], parts[1], parts[2]);
    if year.len() != 4 || month.len() != 2 || !year.chars().all(|c| c.is_ascii_digit()) || !month.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let stem = name.strip_suffix(".md")?;
    let date = stem.to_string();
    let expected = format!("{year}-{month}-");
    if !date.starts_with(&expected) || date.len() != 10 {
        return None;
    }
    if !date[8..].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(date)
}

fn diary_from_heading(content: &str) -> Option<String> {
    // 取文件中第一个一级标题（`# `）行判定；`## ` 之类不算
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("# ") else {
            continue;
        };
        let rest = rest.trim();
        return if rest.len() == 10 && is_iso_date(rest) {
            Some(rest.to_string())
        } else {
            None
        };
    }
    None
}

pub fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

fn relative_path(root: &Path, abs: &Path) -> String {
    abs.strip_prefix(root)
        .unwrap_or(abs)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// `.gitignore` 的最小实现（MVP）：支持 `name`、`dir/`、`*.ext` 三种写法，
/// 不支持的语法（`!` 取反、`**`、中间通配）会被忽略并记入 warnings。
pub struct Ignored {
    dirs: Vec<String>,
    suffixes: Vec<String>,
    names: Vec<String>,
    pub warnings: Vec<String>,
}

impl Ignored {
    pub fn load(root: &Path) -> Self {
        let mut me = Ignored {
            dirs: vec![".git".to_string()],
            suffixes: vec![".tmp".to_string(), ".swp".to_string()],
            names: Vec::new(),
            warnings: Vec::new(),
        };
        let path = root.join(".gitignore");
        let Ok(text) = fs::read_to_string(path) else {
            return me;
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('!') || line.contains("**") {
                me.warnings.push(format!(".gitignore 规则暂不支持: {line}"));
                continue;
            }
            if let Some(dir) = line.strip_suffix('/') {
                me.dirs.push(dir.trim_start_matches('/').to_string());
            } else if let Some(ext) = line.strip_prefix('*') {
                me.suffixes.push(ext.to_string());
            } else {
                me.names.push(line.trim_start_matches('/').to_string());
            }
        }
        me
    }

    /// walkdir 的 filter_entry：目录层面提前剪枝。
    pub fn skip_entry(&self, abs: &Path) -> bool {
        let Some(name) = abs.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return false;
        };
        if self.dirs.iter().any(|d| d == &name) {
            return true;
        }
        self.suffixes.iter().any(|s| name.ends_with(s.as_str()))
    }

    pub fn is_ignored(&self, rel: &str) -> bool {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if self.names.iter().any(|n| n == rel || n == name) {
            return true;
        }
        self.suffixes.iter().any(|s| rel.ends_with(s.as_str()))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diary_by_path() {
        assert_eq!(diary_from_path("2026/10/2026-10-01.md").as_deref(), Some("2026-10-01"));
        assert_eq!(diary_from_path("2026/10/2026-10-02.md").as_deref(), Some("2026-10-02"));
        // 年月与文件名不一致 → 不算日记
        assert_eq!(diary_from_path("2026/10/2026-11-01.md"), None);
        assert_eq!(diary_from_path("raw/供应链.md"), None);
        assert_eq!(diary_from_path("2026-10-01.md"), None);
    }

    #[test]
    fn diary_by_heading_fallback() {
        let content = "# 2026-10-01\n\n## 待办\n";
        assert_eq!(detect_diary("notes/whatever.md", content).as_deref(), Some("2026-10-01"));
        let plain = "# 我的笔记\n";
        assert_eq!(detect_diary("notes/whatever.md", plain), None);
    }

    #[test]
    fn version_is_stable_and_content_sensitive() {
        let a = version_of(1000, "hello");
        let b = version_of(1000, "hello");
        let c = version_of(1000, "hellp");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("1000-"));
    }
}
