// 生成一个「Silver Bullet 风格」的旧笔记目录，用于平替预演
// 特点：布局不统一（有的平铺、有的 YYYY/MM）、日记靠标题判定、含 SB 专属语法、
//       含 todo: 历史写法、含 raw/ 同名冲突、含相对与根基准两种图片引用
// 用法：node tools/legacy-corpus.js <输出目录>
const fs = require('fs');
const path = require('path');

const root = process.argv[2];
if (!root) {
  console.error('用法: node legacy-corpus.js <目录>');
  process.exit(1);
}

// 安全护栏：拒绝在「非空且没有语料标记」的目录上执行（防止误删真实笔记目录）
const MARKER = '.noteschain-corpus';
function assertSafeTarget(dir) {
  if (!fs.existsSync(dir)) return;
  const entries = fs.readdirSync(dir);
  if (entries.length > 0 && !entries.includes(MARKER)) {
    console.error(
      `拒绝执行：${dir} 非空且缺少 ${MARKER} 标记。\n` +
        '若确认这是测试目录，请先手动清空或放入该标记文件；本脚本绝不覆盖来路不明的目录。'
    );
    process.exit(2);
  }
}
assertSafeTarget(root);
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(root, { recursive: true });
fs.writeFileSync(path.join(root, MARKER), 'generated ' + new Date().toISOString() + '\n');

const write = (rel, content, opts = {}) => {
  const file = path.join(root, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  if (opts.bom) {
    fs.writeFileSync(file, Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(content, 'utf8')]));
    return;
  }
  if (opts.crlf) {
    fs.writeFileSync(file, content.replace(/\n/g, '\r\n'), 'utf8');
    return;
  }
  fs.writeFileSync(file, content, 'utf8');
};

// 1) 平铺的日记（靠 `# YYYY-MM-DD` 标题判定）
write('2026-10-01.md', `# 2026-10-01

## 今日日程

- 09:30-10:00 晨会 @线上
- 14:00-15:30 需求评审 #工作/需求

## 待办

- [ ] 整理评审纪要 #工作/需求
- [x] 发会议邀请 ✅ 2026-09-30 18:20

## 记录

今天是十月第一天。
`);

// 2) 另一个平铺日记，带 SB 专属语法与 todo: 历史写法
write('2026-10-02.md', `# 2026-10-02

## 今日日程

- 10:00 一对一

## 待办

- [ ] 对齐季度口径 #工作/供应链/KPI

todo: 这条是老写法，应该进迁移清单
- todo: 这条也是老写法

## 记录

\`\`\`query
page where tag = "工作"
\`\`\`

上面是 Silver Bullet 的查询块，NotesChain 里当普通文本。
`);

// 3) 放在 Daily/ 下的日记（同样靠标题判定）
write('Daily/2026-10-03.md', `# 2026-10-03

## 待办

- [ ] 把周报发出去 #工作
- [ ] 买猫粮 #生活
`);

// 4) 标准 YYYY/MM 布局的日记
write('2026/10/2026-10-04.md', `# 2026-10-04

## 今日日程

- 15:00-17:00 供应链周会 #工作/供应链

## 待办

- [ ] 准备周会材料 #工作/供应链
  - [ ] 拉近 30 天数据
  - [ ] 做一页对比图
`);

// 5) Projects 下的主题笔记（不在 raw/，属于普通笔记）
write('Projects/供应链.md', `# 供应链

## 待办

- [ ] 和采购确认 Q4 到货节奏 #工作/供应链/采购

## 正文

图片（相对文件自身的写法）：![](../assets/a.png)

更多细节见根基准写法：![](assets/b.png)
`);

// 6) raw/ 下同名冲突（PRD 3.5.4）
write('raw/供应链.md', `# 供应链（raw 版）

- [ ] raw 里的一条待办 #工作/供应链
`);
write('raw/子目录/供应链.md', `# 供应链（raw 子目录版）

- [ ] 子目录里的待办 #工作/供应链
`);

// 7) raw/ 普通主题笔记
write('raw/KPI口径.md', `# KPI 口径

- [ ] 对齐指标定义 #工作/供应链/KPI

正文：履约率按承诺日期算。
`);

// 8) 带 BOM 的文件
write('raw/BOM笔记.md', `# BOM 笔记

- [ ] 带 BOM 的待办 #测试
`, { bom: true });

// 9) 带 CRLF 的文件
write('raw/CRLF笔记.md', `# CRLF 笔记

- [ ] 带 CRLF 的待办 #测试
`, { crlf: true });

// 10) 图片资源
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64'
);
fs.mkdirSync(path.join(root, 'assets'), { recursive: true });
fs.writeFileSync(path.join(root, 'assets/a.png'), png);
fs.writeFileSync(path.join(root, 'assets/b.png'), png);

// 12) 带 frontmatter 的日记（Silver Bullet 常见）
write('2026-10-05.md', `---
tags: [工作]
created: 2026-10-05
---

# 2026-10-05

## 待办

- [ ] frontmatter 后面的一条待办 #工作
`);
// 11) 一个只有标题没有待办的文件
write('Inbox/随手记.md', `# 随手记

一些零散想法。
`);

console.log('旧目录语料已生成：' + root);
