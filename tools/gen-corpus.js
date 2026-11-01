// 按 PRD 第 8 章规格生成语料：1000 个 md、约 20 MB、待办约 5000 条
// 用法：node target/gen-corpus3.js <输出目录>
const fs = require('fs');
const path = require('path');

const root = process.argv[2];
if (!root) {
  console.error('用法: node gen-corpus3.js <目录>');
  process.exit(1);
}

const pad = (n) => String(n).padStart(2, '0');
const sizeOf = (lines) => Buffer.byteLength(lines.join('\n'), 'utf8');

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

const start = new Date(2025, 0, 1);
const TARGET = 20000; // 每个文件约 20 KB（字节），1000 个 ≈ 20 MB
let bytes = 0;
let count = 0;

for (let d = 0; d < 730; d++) {
  const date = new Date(start.getTime() + d * 86400000);
  const y = date.getFullYear();
  const m = pad(date.getMonth() + 1);
  const day = pad(date.getDate());
  const iso = `${y}-${m}-${day}`;
  const dir = path.join(root, String(y), m);
  fs.mkdirSync(dir, { recursive: true });

  // 每个日记文件 5 条待办 → 730 * 5 = 3650 条
  const lines = [
    `# ${iso}`,
    '',
    '## 今日日程',
    '',
    '- 09:30-10:00 晨会 @线上',
    `- 15:00-17:00 项目周会 #工作/项目${d % 7}`,
    '',
    '## 待办',
    '',
    `- [ ] 第 ${d} 天的第一件事 #工作/项目${d % 7}`,
    `- [ ] 第 ${d} 天的第二件事 #生活`,
    `- [x] 第 ${d} 天已完成的事 ✅ ${iso} 09:12`,
    '  - [ ] 子项 A',
    '  - [ ] 子项 B',
    '',
    '## 记录',
    '',
    '今天的记录内容，填充一些文字让文件有体积。供应链、采购、履约率、KPI 口径这些都是关键词。',
    '',
    '```sql',
    `SELECT * FROM orders WHERE dt = '${iso}';`,
    '```',
    '',
  ];

  // 只补纯文本，不补待办，避免把待办数量拉爆
  while (sizeOf(lines) + 300 < TARGET) {
    lines.push('补充记录：与供应商对账时发现履约率口径差异，需和财务确认；采购单号与入库单号要能对上；月报口径以财务确认版为准。');
    lines.push('');
  }

  const content = lines.join('\n');
  fs.writeFileSync(path.join(dir, `${iso}.md`), content, 'utf8');
  bytes += Buffer.byteLength(content);
  count++;
}

for (let i = 0; i < 270; i++) {
  const dir = path.join(root, 'raw', `组${i % 9}`);
  fs.mkdirSync(dir, { recursive: true });
  // 每个主题笔记 2 条待办 → 270 * 2 = 540 条
  const lines = [
    `# 主题笔记 ${i}`,
    '',
    '## 待办',
    '',
    `- [ ] 主题 ${i} 的待办 A #项目/组${i % 9}`,
    `- [ ] 主题 ${i} 的待办 B #知识`,
    '',
    '## 正文',
    '',
    `这是第 ${i} 篇主题笔记，讲的是供应链和采购的一些口径问题。关键词：履约率、GMV、对账。`,
    '',
  ];
  while (sizeOf(lines) + 300 < TARGET) {
    lines.push('口径说明：履约率按承诺日期计算，不按实际出库；GMV 含税、不含退款；对账以入库单为准。');
    lines.push('');
  }
  const content = lines.join('\n');
  fs.writeFileSync(path.join(dir, `笔记${i}.md`), content, 'utf8');
  bytes += Buffer.byteLength(content);
  count++;
}

console.log(`生成 ${count} 个文件，共 ${(bytes / 1024 / 1024).toFixed(1)} MB`);
