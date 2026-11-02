/* NotesChain 前端：侧边栏（文件 / 标签）、Markdown 渲染、搜索、首页待办 */

const state = {
  files: null,
  tags: null,
  activePath: null,
  activeTag: null,
  searchStatus: 'all',
  previewMode: true,
  viewSeq: 0,
  scheduleRange: 'week',
  scheduleData: null,
  scheduleTimer: null,
  links: false,
  todoLimit: 1000,
  todoStatus: 'open',
  todoSort: 'default',
  duplicateNames: [],
  pathSet: new Set(),
  tagSort: 'name',
  todoTag: '',
  searchScope: 'all',
  largeAssets: [],
  expandedTags: new Set(),
  leafEls: new Map(),
};

let currentEditor = null;
let saveTimer = null;
let activeFlush = null;
let activeDirty = null;

const main = document.getElementById('main');
const fileTree = document.getElementById('file-tree');
const tagTree = document.getElementById('tag-tree');
const searchInput = document.getElementById('q');

// ---------------------------------------------------------------- 基础工具

function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}

async function api(path, opts) {
  const resp = await fetch(path, opts);
  const data = await resp.json().catch(() => ({}));
  if (!resp.ok) {
    const err = new Error((data && data.error && data.error.message) || `${resp.status} ${resp.statusText}`);
    err.code = data && data.error && data.error.code;
    err.status = resp.status;
    throw err;
  }
  return data;
}

function go(hash) {
  if (location.hash === hash) route();
  else location.hash = hash;
}

// ---------------------------------------------------------------- Markdown 渲染

function inline(raw) {
  const codes = [];
  let s = raw.replace(/`([^`]+)`/g, (_, code) => {
    codes.push(code);
    return '\u0001' + (codes.length - 1) + '\u0001';
  });
  s = esc(s);
  s = s.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (_, alt, src) => {
    const name = alt || src.split('/').pop();
    return `<span class="img-ph" title="${src}">🖼 ${name}</span>`;
  });
  s = s.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, text, href) =>
    /^https?:/i.test(href)
      ? `<a href="${href}" target="_blank" rel="noreferrer">${text}</a>`
      : `<span class="link-ph" title="${href}">${text}</span>`
  );
  s = s.replace(/\[\[([^\]]+)\]\]/g, (_, t) => `<span class="wikilink">${t}</span>`);
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[^*\w])\*([^*\n]+)\*/g, '$1<em>$2</em>');
  s = s.replace(/~~([^~]+)~~/g, '<del>$1</del>');
  s = s.replace(/(^|[\s(（，。；：])#([\w\u3400-\u9fff/-]+)/g, (_, p, tag) => `${p}<span class="tag">#${tag}</span>`);
  // 优先级 !pN：边界与后端 extract_priority 一致（前为行首/空白/括号，后为行尾/空白/标点）
  s = s.replace(/(^|[\s(（，。；：])![pP]([0-9])(?=$|[\s)）\.,，。；：!])/g, (_, p, n) => `${p}<span class="prio p${n}">P${n}</span>`);
  s = s.replace(/\u0001(\d+)\u0001/g, (_, i) => `<code>${esc(codes[Number(i)])}</code>`);
  return s;
}

function splitFrontmatter(text) {
  if (!text.startsWith('---')) return { meta: null, body: text };
  const lines = text.split('\n');
  if (lines[0].trim() !== '---') return { meta: null, body: text };
  const end = lines.findIndex((line, i) => i > 0 && line.trim() === '---');
  if (end < 0) return { meta: null, body: text };
  return { meta: lines.slice(1, end), body: lines.slice(end + 1).join('\n') };
}

function renderMetaCard(metaLines) {
  const rows = [];
  for (const line of metaLines) {
    const at = line.indexOf(':');
    if (at < 0) continue;
    const key = line.slice(0, at).trim();
    const value = line.slice(at + 1).trim();
    if (!key) continue;
    rows.push({ key, value });
  }
  if (!rows.length) return '';
  let html = '<div class="meta-card"><table>';
  for (const row of rows) {
    let value = esc(row.value);
    if (row.key === 'tags') {
      const inner = row.value.replace(/^\[|\]$/g, '').trim();
      value = inner
        ? inner.split(',').map((tag) => `<span class="tag">#${esc(tag.trim())}</span>`).join(' ')
        : '<span class="muted">—</span>';
    }
    html += `<tr><th>${esc(row.key)}</th><td>${value}</td></tr>`;
  }
  return html + '</table></div>';
}

function renderMarkdown(text) {
  const fm = splitFrontmatter(text);
  // body 相对原文件少掉了 frontmatter 的若干行；data-line 要用真实文件行号，便于待办回写
  const fmOffset = fm.meta ? fm.meta.length + 2 : 0;
  text = fm.body;
  const metaHtml = fm.meta ? renderMetaCard(fm.meta) : '';
  const lines = text.split('\n');
  const n = lines.length;
  const out = [];
  let i = 0;

  while (i < n) {
    const line = lines[i];
    const start = i + 1 + fmOffset;

    // 围栏代码块（允许列表项前缀 `- ```sql` 与任意缩进，修复「代码块前面有杠」渲染不出）
    const fence = /^(\s*)(?:[-*+]\s+)?(`{3,}|~{3,})\s*([\w+#.-]*)\s*$/.exec(line);
    if (fence) {
      const marker = fence[2];
      const ch = marker[0] === '`' ? '`' : '~';
      const len = marker.length;
      const buf = [];
      i++;
      const closeRe = new RegExp('^\\s*' + ch + '{' + len + ',}\\s*$');
      while (i < n) {
        if (closeRe.test(lines[i])) {
          i++;
          break;
        }
        buf.push(lines[i]);
        i++;
      }
      out.push(`<pre data-line="${start}"><code>${esc(buf.join('\n'))}</code></pre>`);
      continue;
    }

    if (!line.trim()) {
      i++;
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const level = heading[1].length;
      out.push(`<h${level} data-line="${start}">${inline(heading[2].trim())}</h${level}>`);
      i++;
      continue;
    }

    if (/^\s{0,3}(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      out.push(`<hr data-line="${start}">`);
      i++;
      continue;
    }

    // 表格：本行有 | 且下一行是分隔行
    if (line.includes('|') && i + 1 < n && /^\s*\|?[\s:|-]+\|[\s:|-]*$/.test(lines[i + 1])) {
      const table = parseTable(lines, i);
      out.push(table.html);
      i = table.next;
      continue;
    }

    if (/^\s{0,3}>/.test(line)) {
      const buf = [];
      while (i < n && /^\s{0,3}>/.test(lines[i])) {
        buf.push(lines[i].replace(/^\s{0,3}>\s?/, ''));
        i++;
      }
      out.push(`<blockquote data-line="${start}">${renderMarkdown(buf.join('\n'))}</blockquote>`);
      continue;
    }

    if (/^\s{0,6}([-*+]|\d+\.)\s+/.test(line)) {
      const list = parseList(lines, i, fmOffset);
      out.push(list.html);
      i = list.next;
      continue;
    }

    // 段落
    const buf = [];
    while (i < n) {
      const cur = lines[i];
      if (!cur.trim()) break;
      if (/^\s{0,3}(#{1,6}\s|`{3,}|~{3,}|>)/.test(cur)) break;
      if (/^\s{0,6}([-*+]|\d+\.)\s+/.test(cur)) break;
      if (cur.includes('|') && i + 1 < n && /^\s*\|?[\s:|-]+\|[\s:|-]*$/.test(lines[i + 1])) break;
      buf.push(cur);
      i++;
    }
    if (buf.length) out.push(`<p data-line="${start}">${inline(buf.join('\n'))}</p>`);
    else i++;
  }

  return metaHtml + out.join('\n');
}

function splitRow(line) {
  return line.replace(/^\s*\|/, '').replace(/\|\s*$/, '').split('|').map((s) => s.trim());
}

function parseTable(lines, start) {
  const header = splitRow(lines[start]);
  let i = start + 2;
  const rows = [];
  while (i < lines.length && lines[i].trim() && lines[i].includes('|')) {
    rows.push(splitRow(lines[i]));
    i++;
  }
  let html = `<table data-line="${start + 1}"><thead><tr>`;
  html += header.map((h) => `<th>${inline(h)}</th>`).join('');
  html += '</tr></thead><tbody>';
  for (const row of rows) html += `<tr>${row.map((c) => `<td>${inline(c)}</td>`).join('')}</tr>`;
  return { html: html + '</tbody></table>', next: i };
}

function parseList(lines, start, offset) {
  const items = [];
  let i = start;
  const n = lines.length;
  while (i < n) {
    const m = /^(\s*)([-*+]|\d+\.)\s+(.*)$/.exec(lines[i]);
    if (!m) break;
    items.push({
      indent: m[1].replace(/\t/g, '  ').length,
      ordered: /^\d+\.$/.test(m[2]),
      text: m[3],
      line: i + 1 + (offset || 0),
    });
    i++;
  }
  const tree = toTree(items, 0, -1);
  return { html: renderList(tree.nodes), next: i };
}

function toTree(items, idx, indent) {
  const nodes = [];
  while (idx < items.length && items[idx].indent > indent) {
    const node = Object.assign({}, items[idx], { children: [] });
    idx++;
    if (idx < items.length && items[idx].indent > node.indent) {
      const res = toTree(items, idx, node.indent);
      node.children = res.nodes;
      idx = res.idx;
    }
    nodes.push(node);
  }
  return { nodes, idx };
}

function renderList(nodes) {
  if (!nodes.length) return '';
  const ordered = nodes[0].ordered;
  const tag = ordered ? 'ol' : 'ul';
  let html = `<${tag}>`;
  for (const node of nodes) {
    let cls = '';
    let inner;
    const task = /^\[([ xX])\]\s*(.*)$/.exec(node.text);
    if (task) {
      const done = task[1].toLowerCase() === 'x';
      cls = done ? ' class="task done"' : ' class="task"';
      // 真复选框：预览里可直接勾选（事件在 renderPreviewNodes 里绑定）
      inner = `<input type="checkbox" class="preview-todo" data-line="${node.line}"${done ? ' checked' : ''}>${inline(task[2])}`;
    } else {
      inner = inline(node.text);
    }
    html += `<li${cls} data-line="${node.line}">${inner}`;
    if (node.children.length) html += renderList(node.children);
    html += '</li>';
  }
  return html + `</${tag}>`;
}

// ---------------------------------------------------------------- 视图

function docHead(titleText, rightText, pathText) {
  const head = el('div', 'doc-head');
  head.append(el('span', 'title', titleText));
  if (pathText) head.append(el('span', 'path', pathText));
  if (rightText) head.append(el('span', 'right', rightText));
  return head;
}

function loadingState(text) {
  const wrap = el('div', 'empty-state');
  wrap.append(el('div', '', text || '加载中…'));
  return wrap;
}

function errorState(err) {
  const wrap = el('div', 'empty-state');
  wrap.append(el('h1', '', '出错了'));
  wrap.append(el('div', 'notice', String(err.message || err)));
  return wrap;
}

function startScheduleTicker() {
  if (state.scheduleTimer) clearInterval(state.scheduleTimer);
  state.scheduleTimer = setInterval(() => {
    const onHome = !location.hash || location.hash === '#/' || location.hash === '#';
    if (!onHome || !state.scheduleData) return;
    const old = document.querySelector('.schedule');
    if (!old) return;
    old.replaceWith(buildSchedule(state.scheduleData));
  }, 60000);
}

function stopScheduleTicker() {
  if (state.scheduleTimer) {
    clearInterval(state.scheduleTimer);
    state.scheduleTimer = null;
  }
}
async function refreshTodoSection() {
  const section = document.querySelector('.todos-section');
  if (!section) {
    showHome();
    return;
  }
  const scroll = main.scrollTop;
  const query =
    '/api/todos?status=' + state.todoStatus + '&limit=' + state.todoLimit +
    (state.todoTag ? '&tag=' + encodeURIComponent(state.todoTag) : '');
  try {
    const data = await api(query);
    if (state.todoSort === 'priority') {
      const rank = (item) => (typeof item.priority === 'number' ? item.priority : 99);
      data.items = data.items
        .slice()
        .sort((a, b) => rank(a) - rank(b) || String(a.path).localeCompare(String(b.path)) || a.line - b.line);
    }
    const fresh = buildTodoSection(data, () => {
      state.todoLimit = 50000;
      showHome();
    });
    section.replaceWith(fresh);
    // 两帧后再恢复，等浏览器完成排版，避免被钳到 0
    await new Promise((resolve) => requestAnimationFrame(resolve));
    main.scrollTop = scroll;
    requestAnimationFrame(() => {
      main.scrollTop = scroll;
    });
  } catch (err) {
    /* 保持原样 */
  }
}
async function showHome(preserveScroll) {
  const mySeq = ++state.viewSeq;
  const keepScroll = preserveScroll ? main.scrollTop : 0;
  state.activePath = null;
  state.activeTag = null;
  markActive();
  main.replaceChildren(docHead('首页', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState());
  main.append(wrap);
  try {
    const [todosRaw, git, schedule] = await Promise.all([
      api('/api/todos?status=' + state.todoStatus + '&limit=' + state.todoLimit + (state.todoTag ? '&tag=' + encodeURIComponent(state.todoTag) : '')),
      api('/api/git/status').catch(() => null),
      api('/api/schedule?range=' + state.scheduleRange).catch(() => null),
    ]);
    if (mySeq !== state.viewSeq) return; // 已有更新的视图请求，丢弃本次结果
    const todos = Object.assign({}, todosRaw);
    if (state.todoSort === 'priority') {
      const rank = (item) => (typeof item.priority === 'number' ? item.priority : 99);
      todos.items = todosRaw.items
        .slice()
        .sort((a, b) => rank(a) - rank(b) || String(a.path).localeCompare(String(b.path)) || a.line - b.line);
    }
    setGit(git);
    state.scheduleData = schedule;
    startScheduleTicker();
    const parts = [];
    if (schedule) parts.push(buildSchedule(schedule));
    parts.push(buildTodoSection(todos, () => { state.todoLimit = 50000; showHome(); }));
    wrap.replaceChildren(...parts);
    main.replaceChildren(docHead('首页', `未完成 ${todos.total} 条`), wrap);
    if (preserveScroll) {
      // 轮询刷新首页时保持滚动位置，避免勾选待办后跳回顶部（#8）
      await new Promise((resolve) => requestAnimationFrame(resolve));
      main.scrollTop = keepScroll;
      requestAnimationFrame(() => {
        main.scrollTop = keepScroll;
      });
    }
  } catch (err) {
    wrap.replaceChildren(errorState(err));
  }
}

function buildSchedule(data) {
  const section = el('section', 'schedule');
  const head = el('div', 'section-head');
  head.append(el('h2', '', '日程'));

  const toggle = el('span', 'segmented');
  for (const [value, label] of [['today', '本日'], ['week', '本周'], ['month', '本月']]) {
    const btn = el('button', state.scheduleRange === value ? 'seg active' : 'seg', label);
    btn.type = 'button';
    btn.addEventListener('click', () => {
      state.scheduleRange = value;
      try {
        localStorage.setItem('nc.scheduleRange', value);
      } catch (err) {
        /* ignore */
      }
      showHome();
    });
    toggle.append(btn);
  }
  head.append(toggle);

  const addInput = el('input', 'quick-add');
  addInput.placeholder = '加一条日程，格式：15:00-16:00 会议 @地点（写入今天）';
  addInput.addEventListener('keydown', async (event) => {
    if (event.key !== 'Enter') return;
    const text = addInput.value.trim();
    if (!text) return;
    addInput.disabled = true;
    try {
      await api('/api/schedule/append', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      addInput.value = '';
      state.scheduleRange = 'today';
      await showHome();
      const next = document.querySelector('.schedule .quick-add');
      if (next) next.focus();
    } catch (err) {
      addInput.disabled = false;
      window.alert('添加失败：' + (err.message || err));
    }
  });
  const addRow = el('div', 'quick-add-row');
  addRow.append(addInput);

  section.append(head, addRow);

  const days = (data && data.days) || [];
  if (!days.length) {
    section.append(el('div', 'empty-state', '这一段时间里没有日记文件。'));
    return section;
  }

  const now = new Date();
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
  const toMin = (text) => {
    const parts = String(text).split(':');
    return Number(parts[0]) * 60 + Number(parts[1] || 0);
  };

  for (const day of days) {
    const isPastDay = day.date < todayStr;
    const box = el('div', day.today ? 'day today' : isPastDay ? 'day past' : 'day');
    const dh = el('div', 'day-head');
    dh.style.cursor = 'pointer';
    dh.title = '点击折叠/展开当天日程';
    dh.addEventListener('click', () => box.classList.toggle('collapsed'));

    const caret = el('span', 'day-caret', '▾');
    dh.append(caret);
    if (day.today) dh.append(el('span', 'pill', '今天'));
    dh.append(el('span', 'date', `${day.date.slice(5).replace('-', '/')} ${day.weekday}`));
    if (day.items.length) dh.append(el('span', 'muted', `${day.items.length} 项`));
    box.append(dh);

    if (day.items.length) {
      // 计算状态：进行中置顶，已结束置灰（PRD 3.3.1）
      const decorated = day.items.map((item) => {
        const startMin = toMin(item.start);
        const endMin = item.end ? toMin(item.end) : startMin;
        let status = 'future';
        if (day.today) {
          if (item.end && nowMin >= startMin && nowMin < endMin) status = 'live';
          else if (nowMin >= endMin) status = 'past';
        } else if (isPastDay) {
          status = 'past';
        }
        return { item, status, order: item.end ? 0 : 1 };
      });
      decorated.sort(
        (a, b) => String(a.item.start).localeCompare(String(b.item.start)) || a.order - b.order
      );

      const list = el('ul', 'slots');
      for (const entry of decorated) {
        const item = entry.item;
        const li = el('li', entry.status);
        if (day.path) {
          li.style.cursor = 'pointer';
          li.title = '打开 ' + day.path + ' 第 ' + item.line + ' 行';
          li.addEventListener('click', () =>
            go('/file/' + encodeURIComponent(day.path) + '@' + item.line)
          );
        }
        const time = item.end ? `${item.start}–${item.end}` : item.start;
        li.append(el('span', 'time', time));
        if (entry.status === 'live') li.append(el('span', 'live-pill', '进行中'));
        li.append(el('span', 'what', item.title));
        if (item.place) li.append(el('span', 'place', '@' + item.place));
        for (const tag of item.tags) {
          const chip = el('span', 'tag', '#' + tag);
          chip.style.cursor = 'pointer';
          chip.addEventListener('click', (event) => {
            event.stopPropagation();
            go('/tag/' + encodeURIComponent(tag));
          });
          li.append(chip);
        }
        list.append(li);
      }
      box.append(list);
    } else {
      box.append(el('div', 'muted empty-line', '无日程'));
    }

    if (day.today && day.suspected && day.suspected.length) {
      const hint = el('div', 'unrec muted');
      hint.append(el('span', '', `有 ${day.suspected.length} 行像日程但不在日程段内：`));
      for (const lineNo of day.suspected) {
        const a = el('a', 'suspected-link', `第 ${lineNo} 行`);
        a.href = '#';
        a.addEventListener('click', (event) => {
          event.preventDefault();
          if (day.path) go('/file/' + encodeURIComponent(day.path) + '@' + lineNo);
        });
        hint.append(a);
        hint.append(document.createTextNode(' '));
      }
      box.append(hint);
    }

    if (day.unrecognized && day.unrecognized.length) {
      if (day.today) {
        const unrec = el('div', 'unrec');
        unrec.append(el('span', 'muted', '未识别时间：'));
        unrec.append(el('span', '', day.unrecognized.join('、')));
        box.append(unrec);
      } else {
        box.append(el('div', 'unrec muted', `${day.unrecognized.length} 行未识别`));
      }
    }

    section.append(box);
  }

  return section;
}
function flattenTags(nodes, prefix, out) {
  const result = out || [];
  for (const node of nodes || []) {
    const path = node.path || (prefix ? prefix + '/' + node.name : node.name);
    result.push(path);
    if (node.children && node.children.length) flattenTags(node.children, path, result);
  }
  return result;
}
function buildTodoSection(todos, onShowAll) {
  const section = el('section', 'todos-section');
  const head = el('div', 'section-head');
  head.append(el('h2', '', '待办'));
  head.append(el('span', 'muted', state.todoStatus === 'done' ? `${todos.total} 条已完成` : `${todos.total} 条未完成`));

  const filter = el('select', 'todo-filter');
  filter.append(el('option', '', '全部标签'));
  filter.lastChild.value = '';
  for (const tagPath of flattenTags((state.tags && state.tags.tree) || [])) {
    const option = el('option', '', '#' + tagPath);
    option.value = tagPath;
    filter.append(option);
  }
  filter.value = state.todoTag || '';
  filter.addEventListener('change', () => {
    state.todoTag = filter.value;
    refreshTodoSection();
  });
  head.append(filter);

  const sortSel = el('select', 'todo-filter');
  for (const [value, label] of [['default', '默认排序'], ['priority', '按优先级']]) {
    const opt = el('option', '', label);
    opt.value = value;
    sortSel.append(opt);
  }
  sortSel.value = state.todoSort || 'default';
  sortSel.addEventListener('change', () => {
    state.todoSort = sortSel.value;
    refreshTodoSection();
  });
  head.append(sortSel);

  const toggle = el('span', 'segmented');
  for (const [value, label] of [['open', '未完成'], ['done', '已完成'], ['all', '全部']]) {
    const btn = el('button', state.todoStatus === value ? 'seg active' : 'seg', label);
    btn.type = 'button';
    btn.addEventListener('click', () => {
      state.todoStatus = value;
      refreshTodoSection();
    });
    toggle.append(btn);
  }
  head.append(toggle);

  const addInput = el('input', 'quick-add');
  addInput.placeholder = state.todoStatus === 'done'
    ? '在「未完成」视图下才能添加'
    : '加一条待办，回车保存（#标签 会自动识别）';
  addInput.disabled = state.todoStatus === 'done';
  addInput.addEventListener('keydown', async (event) => {
    if (event.key !== 'Enter') return;
    const text = addInput.value.trim();
    if (!text) return;
    addInput.disabled = true;
    try {
      await api('/api/todos/append', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      addInput.value = '';
      await showHome();
      const next = document.querySelector('.quick-add');
      if (next) next.focus();
    } catch (err) {
      addInput.disabled = false;
      window.alert('添加失败：' + (err.message || err));
    }
  });
  const addRow = el('div', 'quick-add-row');
  addRow.append(addInput);
  section.append(head, addRow, buildTodoTable(todos.items));

  if (todos.total > todos.items.length) {
    const bar = el('div', 'more-bar');
    bar.append(el('span', 'muted', `当前显示 ${todos.items.length} / ${todos.total} 条`));
    if (onShowAll) {
      const btn = el('button', 'seg', '显示全部');
      btn.type = 'button';
      btn.addEventListener('click', onShowAll);
      bar.append(btn);
    }
    section.append(bar);
  }
  return section;
}

function loadColWidths() {
  try {
    // v2：删掉「标签」列后是 4 列（序号 / 勾选 / 事项 / 文件）。
    // 旧键 nc.todoCols 里的宽度是按 5 列存的，混用会让列宽错位，所以换键不迁移。
    const raw = localStorage.getItem('nc.todoCols.v2');
    const saved = raw ? JSON.parse(raw) : null;
    if (!Array.isArray(saved) || saved.length !== 4) return null;
    return saved;
  } catch (err) {
    return null;
  }
}

function makeColResizer(index, colEls) {
  const handle = el('div', 'col-resizer');
  handle.title = '拖动调整列宽';
  handle.addEventListener('mousedown', (event) => {
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startWidth = colEls[index].getBoundingClientRect().width;
    const onMove = (ev) => {
      const width = Math.max(60, Math.round(startWidth + (ev.clientX - startX)));
      colEls[index].style.width = width + 'px';
    };
    const onUp = () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      try {
        localStorage.setItem('nc.todoCols.v2', JSON.stringify(colEls.map((c) => c.style.width)));
      } catch (err) {
        /* 隐私模式下 localStorage 可能不可用 */
      }
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  });
  return handle;
}
function buildPathSet(filesData) {
  const set = new Set();
  for (const group of filesData.groups || []) {
    if (group.kind === 'diary') {
      for (const year of group.years || []) {
        for (const month of year.months || []) {
          for (const entry of month.entries) set.add(entry.path);
        }
      }
    } else if (group.kind === 'note') {
      for (const entry of group.entries) set.add(entry.path);
    }
  }
  state.pathSet = set;
}

function resolveLinkPath(target) {
  const clean = String(target || '').trim();
  if (!clean) return '';
  return clean.includes('/')
    ? (/\.md$/.test(clean) ? clean : clean + '.md')
    : 'raw/' + clean + '.md';
}
/* 待办正文富文本：同时渲染 `行内代码` 与 #标签（标签可点击筛选，代码用等宽样式） */
function appendTodoText(container, raw) {
  const re = /`([^`]+)`|#[\w\u3400-\u9fff/-]+/g;
  let last = 0;
  let match;
  while ((match = re.exec(raw)) !== null) {
    if (match.index > last) container.append(document.createTextNode(raw.slice(last, match.index)));
    if (match[1] !== undefined) {
      // 行内代码
      container.append(el('code', '', match[1]));
    } else {
      const name = match[0].slice(1);
      const chip = el('span', 'tag', match[0]);
      chip.addEventListener('click', (event) => {
        event.stopPropagation();
        go('/tag/' + encodeURIComponent(name));
      });
      container.append(chip);
    }
    last = match.index + match[0].length;
  }
  if (last < raw.length) container.append(document.createTextNode(raw.slice(last)));
}

function buildTodoTable(items) {
  const table = el('table', 'todos');
  const saved = loadColWidths();
  const defaults = ['46px', '38px', 'auto', '176px'];
  const colEls = defaults.map((width, index) => {
    const col = document.createElement('col');
    col.style.width = (saved && saved[index]) || width;
    return col;
  });
  const colgroup = document.createElement('colgroup');
  colEls.forEach((col) => colgroup.append(col));
  table.append(colgroup);

  const head = el('thead');
  const hr = el('tr');
  ['#', '', '事项', '文件'].forEach((label, index) => {
    const th = el('th');
    th.append(el('span', '', label));
    if (index > 1) th.append(makeColResizer(index, colEls));
    hr.append(th);
  });
  head.append(hr);
  const body = el('tbody');

  if (!items.length) {
    const tr = el('tr');
    const td = el('td', '', '没有未完成的待办，清静。');
    td.colSpan = 4;
    tr.append(td);
    body.append(tr);
  }

  const makeRow = (todo, index) => {
    const tr = el('tr');
    tr.dataset.path = todo.path;
    tr.dataset.line = todo.line;
    tr.dataset.rawLine = todo.raw_line;
    tr.dataset.version = todo.version;

    const tdIdx = el('td', 'idx-cell', typeof index === 'number' ? String(index + 1) : '');
    const tdCheck = el('td', 'check');
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = !!todo.done;
    box.addEventListener('change', () => toggleTodo(tr, box));
    tdCheck.append(box);
    if (todo.done) tr.classList.add('is-done');

    const tdText = el('td');
    if (todo.indent) tdText.style.paddingLeft = `${10 + todo.indent * 8}px`;

    if (typeof todo.priority === 'number') {
      tdText.append(el('span', 'prio p' + todo.priority, 'P' + todo.priority));
    }
    const text = el('span', 'text');
    if (todo.indent) text.append(document.createTextNode('· '));
    appendTodoText(text, todo.text);
    tdText.append(text);
    if (todo.done_at) tdText.append(el('span', 'done-at', '✅ ' + todo.done_at));

    const tdFile = el('td');
    const file = el('span', 'file', todo.path.split('/').pop().replace(/\.md$/i, ''));
    file.title = todo.path + ':' + todo.line;
    file.addEventListener('click', () => go('/file/' + encodeURIComponent(todo.path)));
    tdFile.append(file);

    tr.append(tdIdx, tdCheck, tdText, tdFile);
    return tr;
  };

  // 分块渲染：先渲染前 200 行，滚动到哨兵再追加下一批（PRD 4.1 虚拟列表）
  const CHUNK = 200;
  let rendered = 0;
  const sentinel = el('tr', 'sentinel');
  const sentinelCell = el('td');
  sentinelCell.colSpan = 4;
  sentinel.append(sentinelCell);

  const appendChunk = () => {
    const end = Math.min(rendered + CHUNK, items.length);
    const frag = document.createDocumentFragment();
    for (let i = rendered; i < end; i += 1) frag.append(makeRow(items[i], i));
    body.insertBefore(frag, sentinel);
    rendered = end;
    if (rendered >= items.length) {
      sentinel.remove();
      observer.disconnect();
    }
  };

  body.append(sentinel);
  const observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) appendChunk();
    },
    { root: main, rootMargin: '400px' }
  );
  observer.observe(sentinel);
  appendChunk();

  table.append(head, body);
  return table;
}

async function toggleTodo(tr, box) {
  const payload = {
    path: tr.dataset.path,
    line: Number(tr.dataset.line),
    raw_line: tr.dataset.rawLine,
    version: tr.dataset.version,
    done: box.checked,
  };
  tr.classList.add('busy');
  box.disabled = true;
  try {
    await api('/api/todos', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    tr.remove();
    const left = document.querySelectorAll('table.todos tr[data-path]').length;
    const head = main.querySelector('.doc-head .right');
    if (head) head.textContent = `未完成 ${left} 条`;
  } catch (err) {
    tr.classList.remove('busy');
    tr.classList.add('failed');
    box.disabled = false;
    box.checked = !box.checked;
    tr.querySelector('td:nth-child(3)').append(el('div', 'notice', String(err.message || err)));
  }
}

async function showFile(path, line) {
  const mySeq = ++state.viewSeq;
  activeFlush = null;
  activeDirty = null;
  state.activePath = path;
  state.activeTag = null;
  markActive();
  teardownEditor();
  main.replaceChildren(loadingState('打开 ' + path + ' …'));

  let data;
  try {
    data = await api('/api/files/' + encodeURI(path));
    if (mySeq !== state.viewSeq) return; // 慢请求返回时已切到别的页面
  } catch (err) {
    main.replaceChildren(docHead(path.split('/').pop(), '', path), errorState(err));
    return;
  }

  let version = data.version;
  let eol = data.eol || 'lf';
  // 预览与编辑器解耦：liveContent 是「预览显示、也是磁盘最新」的内容；
  // editorStale 表示预览里勾选过待办，隐藏的编辑器文档已过期，切回编辑时需要重建。
  let liveContent = data.content;
  let editorStale = false;
  let autoPreviewOnSave = false;
  let lastSaved = data.content;

  const name = path.split('/').pop();
  const head = docHead(name, '', path);
  const status = el('span', 'status saved', '已保存');
  head.append(status);
  const wrap = el('div', 'wrap');
  const host = el('div', 'editor-host');
  wrap.append(host);
  const layout = el('div', 'file-layout');
  const outline = el('aside', 'outline');
  layout.append(wrap, outline);

  const article = el('article', 'doc preview');
  article.hidden = true;
  wrap.append(article);

  // 预览里勾选待办：直接走后端 PATCH 回写，再把最新内容重新渲染到预览。
  // 关键：绝不对 display:none 的 CodeMirror setValue（会触发测量死循环、整页卡死）。
  const previewToggle = async ({ line, rawLine, done }) => {
    const res = await api('/api/todos', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path, line, raw_line: rawLine, version, done }),
    });
    const fresh = await api('/api/files/' + encodeURI(path));
    version = fresh.version;
    liveContent = fresh.content;
    lastSaved = fresh.content;
    editorStale = true; // 隐藏编辑器内容已过期；切到编辑时再重建
    setPreview(true);
  };

  const previewBtn = el('button', 'seg', '预览');
  previewBtn.type = 'button';
  previewBtn.title = '在渲染视图与编辑视图之间切换（预览里可直接勾选待办；双击正文也可进入编辑）';
  const setPreview = (on) => {
    state.previewMode = on;
    if (on) {
      const content = !editorStale && currentEditor ? currentEditor.getValue() : liveContent;
      liveContent = content;
      article.replaceChildren(...renderPreviewNodes(content, path, { onToggleTodo: previewToggle }));
      renderOutline(outline, content);
      article.hidden = false;
      host.hidden = true;
      previewBtn.textContent = '编辑';
    } else {
      article.hidden = true;
      host.hidden = false;
      previewBtn.textContent = '预览';
      if (currentEditor && editorStale) {
        // 容器已可见后再同步内容，避免隐藏态 setValue 的测量死循环
        currentEditor.setValue(liveContent);
        lastSaved = liveContent;
        editorStale = false;
      }
      if (currentEditor) currentEditor.focus();
    }
  };
  previewBtn.addEventListener('click', () => setPreview(!state.previewMode));
  head.append(previewBtn);

  // 双击预览正文进入编辑（表格内的双击由表格 widget 自己处理）
  article.addEventListener('dblclick', () => {
    if (state.previewMode) {
      autoPreviewOnSave = true; // 双击进入编辑：显式保存后自动回到预览
      setPreview(false);
    }
  });

  const deleteBtn = el('button', 'seg danger', '删除');
  deleteBtn.type = 'button';
  deleteBtn.title = '移到项目根 .trash/（可恢复）';
  deleteBtn.addEventListener('click', async () => {
    if (!window.confirm(`删除「${path}」？会移到 .trash/，可手动恢复。`)) return;
    deleteBtn.disabled = true;
    try {
      await api('/api/files/' + encodeURI(path), { method: 'DELETE' });
      await loadSidebar();
      go('/');
    } catch (err) {
      deleteBtn.disabled = false;
      window.alert('删除失败：' + (err.message || err));
    }
  });
  head.append(deleteBtn);

  main.replaceChildren(head, layout);
  renderOutline(outline, data.content);

  // 以「磁盘上的内容」为基准判断脏：只有编辑器和 lastSaved 不同才需要保存
  let saveChain = Promise.resolve();
  let suppressChange = false;

  const currentText = () => (currentEditor ? currentEditor.getValue() : lastSaved);
  // 预览里勾选过待办（editorStale）时磁盘已是最新、编辑器文档过期，不应判定为脏
  const isDirty = () => !editorStale && currentText() !== lastSaved;

  const setSaved = (content) => {
    lastSaved = content;
    liveContent = content;
    setStatus(status, 'saved', '已保存 ' + timeNow());
    hideConflict(wrap);
  };

  const doSave = async (force = false) => {
    if (!currentEditor || (editorStale && !force)) return true;
    // 用循环而不是「回调 flush」：回调会把新的 then 追加到当前所在的同一条链上，
    // 形成自引用，Promise 永不结算（保存死锁）。
    for (let attempt = 0; attempt < 30; attempt += 1) {
      const content = currentText();
      if (!force && content === lastSaved) return true;
      setStatus(status, 'saving', '保存中…');
      try {
        const res = await api('/api/files/' + encodeURI(path) + (force ? '?force=true' : ''), {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ content, version, eol }),
        });
        version = res.version;
        setSaved(content);
      } catch (err) {
        if (err.code === 'conflict') {
          try {
            const fresh = await api('/api/files/' + encodeURI(path));
            if (fresh.content === content) {
              version = fresh.version;
              setSaved(content);
              if (!isDirty()) return true;
              continue;
            }
          } catch (e) {
            /* 读不到就按真冲突处理 */
          }
          setStatus(status, 'conflict', '有冲突');
          showConflict(wrap, {
            onReload: async () => {
              try {
                const fresh = await api('/api/files/' + encodeURI(path));
                version = fresh.version;
                suppressChange = true;
                currentEditor.setValue(fresh.content);
                setSaved(fresh.content);
              } catch (e) {
                setStatus(status, 'conflict', '重新加载失败');
              }
            },
            onOverwrite: () => flush(true),
          });
          return false;
        }
        setStatus(status, 'conflict', '保存失败');
        showConflict(wrap, { message: String(err.message || err) });
        return false;
      }
      force = false;
      if (!isDirty()) return true;
    }
    return false;
  };
  // 所有保存串成一条链：导航前 await 它，就能保证最后一次输入也落了盘
  const flush = (force = false) => {
    saveChain = saveChain.then(() => doSave(force)).catch(() => false);
    return saveChain;
  };

  activeFlush = () => flush(false);
  activeDirty = () => isDirty();

  const scheduleSave = () => {
    if (suppressChange) {
      suppressChange = false;
      return;
    }
    setStatus(status, 'dirty', '未保存');
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => flush(false), 800);
  };
  const assetFor = (p) =>
    '/api/asset?path=' + encodeURIComponent(p) + '&from=' + encodeURIComponent(path);

  currentEditor = makeEditor(host, data.content, {
    onChange: scheduleSave,
    onSave: () => {
      clearTimeout(saveTimer);
      const wasAuto = autoPreviewOnSave;
      flush(false).then((ok) => {
        if (wasAuto && ok !== false) {
          autoPreviewOnSave = false;
          setPreview(true);
        }
      });
    },
    assetFor,
    links: state.links,
    onOpenLink: openLink,
    onOpenHref: openHref,
    linkExists: (target) => state.pathSet.has(resolveLinkPath(target)),
    previewFor: async (target) => {
      const linkPath = resolveLinkPath(target);
      try {
        const file = await api('/api/files/' + encodeURI(linkPath));
        const lines = file.content.split('\n').filter((line) => line.trim()).slice(0, 8);
        const title = esc(linkPath);
        const body = esc(lines.join('\n')).slice(0, 600);
        return `<div class="hover-title">${title}</div><pre class="hover-body">${body}</pre>`;
      } catch (err) {
        return `<div class="hover-title">${esc(linkPath)}</div><div class="hover-missing">该笔记还不存在，点击可新建</div>`;
      }
    },
    onToggleTodo: async (info) => {
      // 先把正文改动落盘（否则 PATCH 之后整篇保存会把完成标记覆盖掉）
      const flushed = await flush(false);
      if (flushed === false) return; // 正文没存成功（冲突），不要继续改文件
      const res = await api('/api/todos', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, line: info.line, raw_line: info.rawLine, version, done: info.done }),
      });
      version = res.version;
      const fresh = await api('/api/files/' + encodeURI(path));
      suppressChange = true;
      currentEditor.setValue(fresh.content);
      setSaved(fresh.content);
    },
  });
  currentEditor.focus();
  state.previewMode = true;
  setPreview(true);
  if (line) highlightLine(line);
  if (state.links) {
    mountBacklinks(wrap, path);
    mountOutgoing(wrap, path);
  }
}

async function mountOutgoing(wrap, path) {
  const panel = el('section', 'backlinks');
  const head = el('div', 'section-head');
  head.append(el('h2', '', '出链'));
  panel.append(head);
  const body = el('div', 'muted', '读取中…');
  panel.append(body);
  wrap.append(panel);
  try {
    const data = await api('/api/links?path=' + encodeURIComponent(path));
    body.replaceChildren();
    if (!data.total) {
      body.textContent = '这篇没有指向别处的双链。';
      return;
    }
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      li.append(el('div', 'meta', `第 ${item.line} 行`));
      const resolved = item.resolved ? item.resolved : '未命中';
      li.append(el('div', 'snippet', `[[${item.target}]] → ${resolved}`));
      if (item.resolved) {
        li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.resolved + '.md')));
      }
      list.append(li);
    }
    body.replaceChildren(list);
  } catch (err) {
    body.textContent = '读取失败：' + (err.message || err);
  }
}
async function mountBacklinks(wrap, path) {
  const panel = el('section', 'backlinks');
  const head = el('div', 'section-head');
  head.append(el('h2', '', '反向链接'));
  panel.append(head);
  const body = el('div', 'muted', '读取中…');
  panel.append(body);
  wrap.append(panel);
  try {
    const data = await api('/api/backlinks?path=' + encodeURIComponent(path));
    body.replaceChildren();
    if (!data.total) {
      body.textContent = '没有其他笔记链接到这里。';
      return;
    }
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      li.append(el('div', 'meta', `${item.path}:${item.line}`));
      li.append(el('div', 'snippet', item.text));
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path) + '@' + item.line));
      list.append(li);
    }
    body.replaceChildren(list);
  } catch (err) {
    body.textContent = '读取失败：' + (err.message || err);
  }
}

function stripDoneMarker(line) {
  const text = String(line || '').replace(/\s+$/, '');
  if (!/^\s*- \[[xX]\]/.test(text)) return text;
  const at = text.indexOf('✅');
  return at >= 0 ? text.slice(0, at).trimEnd() : text;
}

function renderPreviewNodes(content, filePath, opts) {
  const article = document.createElement('div');
  article.innerHTML = renderMarkdown(content);
  const lines = content.split('\n');
  const assetFor = (p) => '/api/asset?path=' + encodeURIComponent(p) + '&from=' + encodeURIComponent(filePath);

  article.querySelectorAll('.img-ph').forEach((span) => {
    const src = span.getAttribute('title') || '';
    const img = document.createElement('img');
    img.className = 'preview-img';
    img.alt = span.textContent;
    img.src = assetFor(src);
    img.addEventListener('error', () => {
      span.textContent = '🖼 ' + (span.textContent || src);
      img.replaceWith(span);
    });
    span.replaceWith(img);
  });

  article.querySelectorAll('.link-ph').forEach((span) => {
    const href = span.getAttribute('title') || '';
    const link = document.createElement('a');
    link.textContent = span.textContent;
    link.href = '#';
    link.addEventListener('click', (event) => {
      event.preventDefault();
      openHref(href);
    });
    span.replaceWith(link);
  });

  article.querySelectorAll('.wikilink').forEach((span) => {
    const target = span.textContent;
    span.style.cursor = 'pointer';
    span.addEventListener('click', () => openLink(target));
  });

  // 预览里的待办复选框：点击 → 回调里走 PATCH 回写（避免对隐藏编辑器 setValue）
  if (opts && typeof opts.onToggleTodo === 'function') {
    article.querySelectorAll('.preview-todo').forEach((box) => {
      const lineNo = Number(box.getAttribute('data-line'));
      if (!lineNo) return;
      box.addEventListener('change', async () => {
        const rawLine = stripDoneMarker(lines[lineNo - 1] || '');
        box.disabled = true;
        try {
          await opts.onToggleTodo({ line: lineNo, rawLine, done: box.checked });
        } catch (err) {
          box.checked = !box.checked;
          box.disabled = false;
        }
      });
    });
  }

  return Array.from(article.childNodes);
}
function renderOutline(container, content) {
  container.replaceChildren();
  container.append(el('div', 'outline-head', '大纲'));

  const lines = content.split('\n');
  let inFence = false;
  const items = [];
  lines.forEach((line, index) => {
    if (/^\s{0,3}(`{3,}|~{3,})/.test(line)) {
      inFence = !inFence;
      return;
    }
    if (inFence) return;
    const m = /^(#{1,6})\s+(.+?)\s*$/.exec(line);
    if (m) items.push({ level: m[1].length, text: m[2].replace(/[#*`]/g, '').trim(), line: index + 1 });
  });

  if (!items.length) {
    container.append(el('div', 'outline-empty', '这篇没有标题'));
    return;
  }
  for (const item of items) {
    const link = el('a', 'outline-item level-' + item.level, item.text);
    link.href = '#';
    link.title = item.text;
    link.addEventListener('click', (event) => {
      event.preventDefault();
      jumpToLine(item.line);
    });
    container.append(link);
  }
}

function jumpToLine(lineNumber) {
  if (!currentEditor || !currentEditor.view) return;
  const view = currentEditor.view;
  const line = Math.min(Math.max(1, lineNumber), view.state.doc.lines);
  const pos = view.state.doc.line(line).from;
  view.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
  view.focus();
}
function makeEditor(host, content, options) {
  const { onChange, onSave } = options;
  if (window.NotesChainEditor && typeof window.NotesChainEditor.create === 'function') {
    return window.NotesChainEditor.create(Object.assign({ parent: host, doc: content }, options));
  }
  const area = document.createElement('textarea');
  area.className = 'editor-fallback';
  area.value = content;
  area.spellcheck = false;
  area.addEventListener('input', () => onChange(area.value));
  area.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      onSave();
    }
  });
  host.append(area);
  return {
    getValue: () => area.value,
    setValue: (text) => {
      area.value = text;
    },
    focus: () => area.focus(),
    destroy: () => area.remove(),
  };
}

function teardownEditor() {
  clearTimeout(saveTimer);
  if (currentEditor) {
    try {
      currentEditor.destroy();
    } catch (err) {
      /* ignore */
    }
    currentEditor = null;
  }
}

function setStatus(node, cls, text) {
  node.className = 'status ' + cls;
  node.textContent = text;
}

function timeNow() {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

function showConflict(wrap, handlers) {
  hideConflict(wrap);
  const bar = el('div', 'conflict-bar');
  bar.append(el('span', '', handlers.message || '文件已被外部修改，请选择处理方式：'));
  if (handlers.onReload) {
    const reload = el('button', 'seg', '重新加载');
    reload.addEventListener('click', handlers.onReload);
    bar.append(reload);
  }
  if (handlers.onOverwrite) {
    const overwrite = el('button', 'seg active', '强制覆盖');
    overwrite.addEventListener('click', handlers.onOverwrite);
    bar.append(overwrite);
  }
  wrap.prepend(bar);
}

function hideConflict(wrap) {
  const existing = wrap.querySelector('.conflict-bar');
  if (existing) existing.remove();
}

function highlightLine(lineNumber) {
  if (!currentEditor || !currentEditor.view) return;
  const view = currentEditor.view;
  const number = Math.min(Math.max(1, lineNumber), view.state.doc.lines);
  const line = view.state.doc.line(number);
  view.dispatch({ selection: { anchor: line.from, head: line.to }, scrollIntoView: true });
  view.focus();
}
async function showSearch(q) {
  const mySeq = ++state.viewSeq;
  state.activePath = null;
  markActive();
  main.replaceChildren(docHead('搜索', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState(`搜索「${q}」…`));
  main.append(wrap);
  try {
    const data = await api('/api/search?q=' + encodeURIComponent(q) + '&status=' + state.searchStatus + '&scope=' + state.searchScope);
    if (mySeq !== state.viewSeq) return;
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      const meta = el('div', 'meta', `${item.path}:${item.line}`);
      meta.append(el('span', 'kind', kindLabel(item.kind)));
      li.append(meta);
      const isCode = item.kind === 'code';
      if (item.before) li.append(el('div', 'ctx', item.before));
      const snippet = el('div', isCode ? 'snippet clamped' : 'snippet');
      snippet.innerHTML = highlight(item.snippet, q);
      li.append(snippet);
      if (item.after) li.append(el('div', 'ctx', item.after));
      if (isCode) {
        const toggle = el('button', 'seg tiny', '展开');
        toggle.type = 'button';
        toggle.addEventListener('click', (event) => {
          event.stopPropagation();
          snippet.classList.toggle('clamped');
          toggle.textContent = snippet.classList.contains('clamped') ? '展开' : '收起';
        });
        li.append(toggle);
      }
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path) + (item.line ? '@' + item.line : '')));
      list.append(li);
    }
    if (!data.items.length) {
      wrap.replaceChildren(el('div', 'empty-state', `没有命中「${q}」的内容。`));
    } else {
      wrap.replaceChildren(list);
    }
    const head = docHead('搜索', `${data.total} 处命中`, q);
    const bodyOnly = el('button', state.searchScope === 'text' ? 'seg active' : 'seg', '仅看正文');
    bodyOnly.type = 'button';
    bodyOnly.title = '排除代码块里的命中';
    bodyOnly.addEventListener('click', () => {
      state.searchScope = state.searchScope === 'text' ? 'all' : 'text';
      showSearch(q);
    });
    head.append(bodyOnly);
    head.append(statusToggle(() => showSearch(q)));
    main.replaceChildren(head, wrap);
  } catch (err) {
    wrap.replaceChildren(errorState(err));
  }
}

function statusToggle(onChange) {
  const box = el('span', 'segmented');
  for (const [value, label] of [['all', '全部'], ['open', '未完成'], ['done', '已完成']]) {
    const btn = el('button', state.searchStatus === value ? 'seg active' : 'seg', label);
    btn.type = 'button';
    btn.addEventListener('click', () => {
      state.searchStatus = value;
      onChange();
    });
    box.append(btn);
  }
  return box;
}

async function showTag(path) {
  state.activeTag = path;
  state.activePath = null;
  markActive();
  main.replaceChildren(docHead('标签', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState(`标签 #${path} …`));
  main.append(wrap);
  try {
    const [files, todos] = await Promise.all([
      api('/api/search?tag=' + encodeURIComponent(path)),
      api('/api/todos?tag=' + encodeURIComponent(path) + '&limit=200').catch(() => ({ total: 0, items: [] })),
    ]);
    const parts = [];
    if (todos.total) {
      parts.push(buildTodoSection(todos, null));
    }
    const list = el('ul', 'results');
    for (const item of files.items) {
      const li = el('li');
      // 显示标签所在行（后端已改为返回命中标签的那一行，而不是首行/`---`）
      const where = item.line && item.line > 0 ? ':' + item.line : '';
      li.append(el('div', 'meta', item.path.replace(/\.md$/i, '') + where));
      li.append(el('div', 'snippet', item.snippet));
      const jump = item.line && item.line > 1 ? '@' + item.line : '';
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path) + jump));
      list.append(li);
    }
    if (!files.items.length) parts.push(el('div', 'empty-state', '这个标签下还没有笔记。'));
    else parts.push(list);
    wrap.replaceChildren(...parts);
    main.replaceChildren(docHead('#' + path, `${files.total} 篇 · ${todos.total} 条待办`, ''), wrap);
  } catch (err) {
    wrap.replaceChildren(errorState(err));
  }
}

function kindLabel(kind) {
  return { code: '代码块', todo: '待办', text: '正文', file: '文件' }[kind] || kind;
}

function highlight(text, needle) {
  const safe = esc(text);
  if (!needle) return safe;
  const pattern = esc(needle).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  try {
    return safe.replace(new RegExp(pattern, 'gi'), (m) => `<mark>${m}</mark>`);
  } catch {
    return safe;
  }
}

// ---------------------------------------------------------------- 侧边栏

async function loadSidebar() {
  const [files, tags] = await Promise.all([api('/api/files'), api('/api/tags')]);
  buildPathSet(files);
  state.files = files;
  state.tags = tags;
  renderFileTree(files);
  renderTagTree(tags.tree);
}

function renderFileTree(data) {
  fileTree.replaceChildren();
  state.leafEls.clear();
  const diary = (data.groups || []).find((g) => g.kind === 'diary');
  const notes = (data.groups || []).find((g) => g.kind === 'note');

  let total = 0;
  for (const year of (diary && diary.years) || []) {
    const yearBox = el('details');
    const yearSum = el('summary');
    yearSum.append(el('span', '', year.year));
    let yearCount = 0;
    for (const month of year.months) yearCount += month.entries.length;
    yearSum.append(el('span', 'n', String(yearCount)));
    yearBox.append(yearSum);

    for (const month of year.months) {
      const monthBox = el('details');
      const monthSum = el('summary');
      monthSum.append(el('span', '', `${month.month} 月`));
      monthSum.append(el('span', 'n', String(month.entries.length)));
      monthBox.append(monthSum);

      for (const entry of month.entries) {
        monthBox.append(fileLeaf(entry.path, entry.path.split('/').pop().replace(/\.md$/, ''), entry.todos));
        total++;
      }
      yearBox.append(monthBox);
    }
    fileTree.append(yearBox);
  }

  if (notes && notes.entries.length) {
    const box = el('details');
    const sum = el('summary');
    sum.append(el('span', '', '主题笔记'));
    sum.append(el('span', 'n', String(notes.entries.length)));
    box.append(sum);
    for (const entry of notes.entries) {
      box.append(fileLeaf(entry.path, entry.path.split('/').pop().replace(/\.md$/, ''), entry.todos));
      total++;
    }
    fileTree.append(box);
  }

  if (!total) fileTree.append(el('div', 'empty-state', '还没有 Markdown 文件。'));
  document.getElementById('file-count').textContent = String(total);
}

function fileLeaf(path, label, todos) {
  const leaf = el('a', 'leaf');
  leaf.append(el('span', '', label));
  // 同名笔记（raw/ 与子目录）只显示 basename 会分不清，补上所在目录
  const dup = (state.duplicateNames || []).some((d) => d.name === label);
  if (dup) leaf.append(el('span', 'leaf-hint', path.split('/').slice(0, -1).join('/') + '/'));
  leaf.title = path;
  if (todos) leaf.append(el('span', 'n', String(todos)));
  leaf.addEventListener('click', () => go('/file/' + encodeURIComponent(path)));
  state.leafEls.set(path, leaf);
  return leaf;
}

function renderTagTree(nodes) {
  tagTree.replaceChildren();
  const panelHead = document.querySelector('#tag-panel-head');
  if (panelHead && !panelHead.querySelector('.mini')) {
    const btn = el('button', 'mini', state.tagSort === 'todos' ? '按待办' : '按名称');
    btn.type = 'button';
    btn.title = '切换标签排序';
    btn.addEventListener('click', () => {
      state.tagSort = state.tagSort === 'todos' ? 'name' : 'todos';
      btn.textContent = state.tagSort === 'todos' ? '按待办' : '按名称';
      try {
        localStorage.setItem('nc.tagSort', state.tagSort);
      } catch (err) {
        /* ignore */
      }
      renderTagTree(state.tags ? state.tags.tree : []);
    });
    panelHead.append(btn);
  }
  if (!nodes || !nodes.length) {
    tagTree.append(el('div', 'empty-state', '还没有标签。'));
    return;
  }

  const walk = (list, depth, container) => {
    for (const node of list) {
      const hasChildren = node.children && node.children.length > 0;
      const row = el('div', 'tag-node');
      row.style.paddingLeft = `${6 + depth * 12}px`;
      row.title = '#' + node.path;
      row.dataset.tag = node.path;

      const caret = el('span', 'caret', hasChildren ? '▸' : '');
      if (hasChildren) {
        const expanded = state.expandedTags.has(node.path);
        caret.classList.toggle('open', expanded);
        caret.addEventListener('click', (event) => {
          event.stopPropagation();
          const sub = row.nextElementSibling;
          if (!sub) return;
          if (state.expandedTags.has(node.path)) state.expandedTags.delete(node.path);
          else state.expandedTags.add(node.path);
          sub.hidden = !state.expandedTags.has(node.path);
          caret.classList.toggle('open', !sub.hidden);
        });
      }

      row.append(caret);
      const nameSpan = el('span', 'tag-name', node.name);
      nameSpan.title = '#' + node.path;
      row.append(nameSpan);
      const openBtn = el('span', 'tag-open', '查看');
      openBtn.title = '查看 #' + node.path;
      openBtn.addEventListener('click', (event) => {
        event.stopPropagation();
        go('/tag/' + encodeURIComponent(node.path));
      });
      row.append(openBtn);
      row.append(el('span', 'n', `${node.notes} 篇 / ${node.todos} 待办`));
      row.addEventListener('click', () => { const sub = row.nextElementSibling; if (!hasChildren || !sub) { go('/tag/' + encodeURIComponent(node.path)); return; } if (state.expandedTags.has(node.path)) state.expandedTags.delete(node.path); else state.expandedTags.add(node.path); sub.hidden = !state.expandedTags.has(node.path); caret.classList.toggle('open', !sub.hidden); });
      container.append(row);

      if (hasChildren) {
        const sub = el('div', 'tag-children');
        sub.hidden = !state.expandedTags.has(node.path);
        container.append(sub);
        walk(node.children, depth + 1, sub);
      }
    }
  };

  const sorter = (a, b) =>
    state.tagSort === 'todos'
      ? b.todos - a.todos || a.name.localeCompare(b.name)
      : a.name.localeCompare(b.name);
  const sortNodes = (list) =>
    list
      .slice()
      .sort(sorter)
      .map((node) => Object.assign({}, node, { children: node.children ? sortNodes(node.children) : [] }));

  walk(sortNodes(nodes), 0, tagTree);
  document.getElementById('tag-count').textContent = String(nodes.length);
}
function expandToActive() {
  const leaf = state.activePath ? state.leafEls.get(state.activePath) : null;
  if (!leaf) return;
  let node = leaf.parentElement;
  while (node) {
    if (node.tagName === 'DETAILS') node.open = true;
    node = node.parentElement;
  }
  leaf.scrollIntoView({ block: 'nearest' });
}

function markActive() {
  for (const [path, leaf] of state.leafEls) leaf.classList.toggle('active', path === state.activePath);
  expandToActive();
  for (const row of tagTree.querySelectorAll('.tag-node')) {
    row.classList.toggle('active', row.dataset.tag === state.activeTag);
  }
}

function setGit(git) {
  const box = document.getElementById('git');
  if (!git) {
    box.textContent = '';
    return;
  }
  if (!git.available) {
    box.textContent = git.disabled_by_user ? '版本历史已关闭' : '未检测到 git';
    return;
  }
  const parts = [];
  if (git.disabled) parts.push('自动提交已停止');
  parts.push(git.last_commit ? '上次提交 ' + git.last_commit.slice(0, 16).replace('T', ' ') : '尚无提交');
  if (git.pending_files) parts.push('待提交 ' + git.pending_files);
  box.textContent = parts.join(' · ');
  const tips = [];
  if (git.repo_root) tips.push('仓库根：' + git.repo_root);
  tips.push('仅本地版本历史，无远程备份');
  if (git.last_error) tips.push('最近错误：' + git.last_error);
  box.title = tips.join('\n');
}

// ---------------------------------------------------------------- 路由

async function route() {
  stopScheduleTicker();
  // 切换页面前先把未保存的改动落盘（PRD 5.3：保存先于跳转）
  if (activeDirty && activeDirty() && activeFlush) {
    const flushFn = activeFlush;
    const ok = await flushFn();
    if (ok === false) return; // 有冲突/保存失败：留在当前页，别丢内容
    // 只有确认保存成功才注销句柄；否则下次导航仍需要 flush 保护
    activeFlush = null;
    activeDirty = null;
  }
  const raw = location.hash.slice(1);
  if (raw.startsWith('/file/')) {
    const rest = decodeURIComponent(raw.slice(6));
    const at = rest.lastIndexOf('@');
    if (at > 0) return showFile(rest.slice(0, at), Number(rest.slice(at + 1)));
    return showFile(rest);
  }
  if (raw.startsWith('/search/')) return showSearch(decodeURIComponent(raw.slice(8)));
  if (raw.startsWith('/tag/')) return showTag(decodeURIComponent(raw.slice(5)));
  if (raw.startsWith('/migration')) return showMigration();
  if (raw.startsWith('/history')) return showHistory();
  if (raw.startsWith('/commit/')) return showCommit(raw.slice(8));
  return showHome();
}

// Ctrl/Cmd+K 聚焦搜索；非输入态按 / 也可以
document.addEventListener('keydown', (event) => {
  const tag = (event.target.tagName || '').toLowerCase();
  const typing = tag === 'input' || tag === 'textarea' || event.target.isContentEditable;
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault();
    searchInput.focus();
    searchInput.select();
    return;
  }
  if (event.key === '/' && !typing) {
    event.preventDefault();
    searchInput.focus();
  }
});
let searchTimer = null;
searchInput.addEventListener('input', () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    const q = searchInput.value.trim();
    if (q) go('/search/' + encodeURIComponent(q));
    else go('/');
  }, 280);
});

window.addEventListener('hashchange', route);

function normalizeRel(base, href) {
  const segs = [];
  for (const seg of ((base ? base + '/' : '') + href).split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') segs.pop();
    else segs.push(seg);
  }
  return segs.join('/');
}

function openHref(href) {
  const clean = String(href || '').split('#')[0].trim();
  if (!clean) return;
  if (/^https?:/i.test(clean)) {
    window.open(clean, '_blank', 'noreferrer');
    return;
  }
  if (!/\.md$/i.test(clean)) return;
  const base = state.activePath ? state.activePath.split('/').slice(0, -1).join('/') : '';
  // 双轨：先按当前文件相对路径，找不到再按 notes 根（与图片解析一致）
  const relative = normalizeRel(base, clean);
  const rooted = normalizeRel('', clean);
  const path = state.pathSet.has(relative) ? relative : rooted;
  go('/file/' + encodeURIComponent(path));
}
async function openLink(target) {
  const clean = String(target || '').trim();
  if (!clean) return;
  if (!clean.includes('/') && state.duplicateNames.some((d) => d.name === clean)) {
    window.alert(`「${clean}」在 raw/ 下有多份同名笔记，请改用路径形式，如 [[raw/子目录/${clean}]]`);
    return;
  }
  const path = clean.includes('/')
    ? (/\.md$/.test(clean) ? clean : clean + '.md')
    : 'raw/' + clean + '.md';
  try {
    await api('/api/files/' + encodeURI(path));
    go('/file/' + encodeURIComponent(path));
  } catch (err) {
    if (!window.confirm(`「${path}」还不存在，要新建吗？`)) return;
    try {
      await api('/api/notes/new', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path }),
      });
      await loadSidebar();
      go('/file/' + encodeURIComponent(path));
    } catch (e) {
      window.alert('新建失败：' + (e.message || e));
    }
  }
}
// ---------------------------------------------------------------- todo: 迁移清单

async function showMigration() {
  state.activePath = null;
  state.activeTag = null;
  markActive();
  main.replaceChildren(docHead('迁移清单', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState('扫描 todo: 写法…'));
  main.append(wrap);
  try {
    const data = await api('/api/migration/todo');
    if (!data.total) {
      wrap.replaceChildren(el('div', 'empty-state', '没有发现 todo: 写法的历史待办。'));
      return;
    }
    const tip = el('div', 'muted', '程序不会自动改写这些行。需要它们进入首页待办，请手工改成 - [ ] 语法。');
    tip.style.padding = '0 0 12px';
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      li.append(el('div', 'meta', `${item.path}:${item.line}`));
      li.append(el('div', 'snippet', item.text));
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path) + '@' + item.line));
      list.append(li);
    }
    wrap.replaceChildren(tip, list);
    main.replaceChildren(docHead('迁移清单', `${data.total} 条`, ''), wrap);
  } catch (err) {
    wrap.replaceChildren(errorState(err));
  }
}
// ---------------------------------------------------------------- 历史

async function showHistory() {
  state.activePath = null;
  state.activeTag = null;
  markActive();
  main.replaceChildren(docHead('历史', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState('读取提交历史…'));
  main.append(wrap);
  try {
    const data = await api('/api/git/log?limit=100');
    if (!data.available) {
      wrap.replaceChildren(el('div', 'empty-state', data.reason || '未启用 git，没有版本历史。'));
      return;
    }
    const list = el('ul', 'history-list');
    for (const commit of data.commits) {
      const li = el('li');
      li.append(el('span', 'time', commit.time));
      li.append(el('span', 'subject', commit.subject));
      li.append(el('span', 'hash', commit.short));
      li.addEventListener('click', () => go('/commit/' + commit.hash));
      list.append(li);
    }
    if (!data.commits.length) wrap.replaceChildren(el('div', 'empty-state', '还没有提交。'));
    else wrap.replaceChildren(list);
    main.replaceChildren(docHead('历史', `${data.commits.length} 条`, ''), wrap);
  } catch (err) {
    wrap.replaceChildren(errorState(err));
  }
}

async function showCommit(hash) {
  state.activePath = null;
  markActive();
  main.replaceChildren(docHead('提交', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState('读取 diff…'));
  main.append(wrap);
  try {
    const data = await api('/api/git/show?hash=' + encodeURIComponent(hash));
    const files = Array.from(new Set(
      Array.from(data.patch.matchAll(/^\+\+\+ b\/(.+?)\r?$/gm)).map((m) => m[1].trim())
    ));
    const bar = el('div', 'restore-bar');
    if (files.length) {
      bar.append(el('span', 'muted', '该提交涉及：'));
      for (const file of files) {
        const btn = el('button', 'seg', '恢复 ' + file);
        btn.type = 'button';
        btn.addEventListener('click', async () => {
          if (!window.confirm(`把「${file}」恢复到该提交的版本？当前内容会被覆盖。`)) return;
          btn.disabled = true;
          try {
            await api('/api/git/restore', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ hash, path: file }),
            });
            await loadSidebar();
            btn.textContent = '已恢复';
          } catch (err) {
            btn.disabled = false;
            window.alert('恢复失败：' + (err.message || err));
          }
        });
        bar.append(btn);
      }
    }
    const pre = el('pre', 'patch');
    pre.textContent = data.patch || '（该提交没有文本差异）';
    wrap.replaceChildren(bar, pre);
    main.replaceChildren(docHead('提交', hash.slice(0, 8), ''), wrap);
  } catch (err) {
    wrap.replaceChildren(errorState(err));
  }
}
// ---------------------------------------------------------------- 外部改动轮询

let lastRevision = null;

async function pollRevision() {
  try {
    const data = await api('/api/revision');
    if (lastRevision === null) {
      lastRevision = data.revision;
    } else if (data.revision !== lastRevision) {
      lastRevision = data.revision;
      await refreshFromDisk();
    }
    setGit(await api('/api/git/status').catch(() => null));
  } catch (err) {
    /* 服务重启或网络抖动：下一轮再试 */
  }
  setTimeout(pollRevision, 5000);
}

async function refreshFromDisk() {
  try {
    await loadSidebar();
  } catch (err) {
    /* 侧边栏刷新失败不影响主区域 */
  }
  if (!state.activePath) {
    await showHome(true); // 轮询触发的首页刷新：保持滚动位置
    return;
  }
  // 编辑中不覆盖用户内容；未编辑则只在磁盘版本真的变了时重载
  if (activeDirty && activeDirty()) return;
  try {
    const fresh = await api('/api/files/' + encodeURI(state.activePath));
    const current = currentEditor ? currentEditor.getValue() : null;
    if (current !== null && current === fresh.content) return;
    await showFile(state.activePath);
  } catch (err) {
    /* 文件可能被删了，交给下一次导航处理 */
  }
}

function todayPath() {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, '0');
  const d = String(now.getDate()).padStart(2, '0');
  return `${y}/${m}/${y}-${m}-${d}.md`;
}

const todayBtn = document.getElementById('today-link');
if (todayBtn) {
  todayBtn.addEventListener('click', (event) => {
    event.preventDefault();
    go('/file/' + encodeURIComponent(todayPath()));
  });
}

const rebuildBtn = document.getElementById('rebuild');
if (rebuildBtn) {
  rebuildBtn.addEventListener('click', async () => {
    rebuildBtn.disabled = true;
    try {
      const res = await api('/api/index/rebuild', { method: 'POST' });
      await loadSidebar();
      await route();
      rebuildBtn.textContent = `已重建 ${res.files}`;
      setTimeout(() => (rebuildBtn.textContent = '重建索引'), 2000);
    } catch (err) {
      window.alert('重建失败：' + (err.message || err));
    } finally {
      rebuildBtn.disabled = false;
    }
  });
}

const wideBtn = document.getElementById('wide-toggle');
if (wideBtn) {
  const applyWide = (on) => {
    document.body.classList.toggle('wide', on);
    wideBtn.textContent = on ? '常规宽度' : '宽屏';
  };
  let wide = false;
  try {
    wide = localStorage.getItem('nc.wide') === '1';
  } catch (err) {
    wide = false;
  }
  applyWide(wide);
  wideBtn.addEventListener('click', () => {
    wide = !document.body.classList.contains('wide');
    applyWide(wide);
    try {
      localStorage.setItem('nc.wide', wide ? '1' : '0');
    } catch (err) {
      /* ignore */
    }
  });
}

/* 侧边栏「⋯」菜单：历史 / 宽屏 / 刷新 / 重建索引 */
const moreToggle = document.getElementById('more-toggle');
const moreMenu = document.getElementById('more-menu');
if (moreToggle && moreMenu) {
  const setMoreOpen = (open) => {
    moreMenu.hidden = !open;
    moreToggle.setAttribute('aria-expanded', String(open));
  };
  moreToggle.addEventListener('click', (event) => {
    event.stopPropagation();
    setMoreOpen(moreMenu.hidden);
  });
  moreMenu.addEventListener('click', (event) => {
    if (event.target.closest('.quick')) setMoreOpen(false);
  });
  document.addEventListener('click', (event) => {
    if (!moreMenu.hidden && !event.target.closest('.quicknav')) setMoreOpen(false);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !moreMenu.hidden) setMoreOpen(false);
  });
  window.addEventListener('hashchange', () => setMoreOpen(false));
}

async function openNewNoteDialog() {
  let templates = [];
  try {
    const data = await api('/api/templates');
    templates = data.templates || [];
  } catch (err) {
    templates = [];
  }

  const mask = el('div', 'modal-mask');
  const box = el('div', 'modal new-note-modal');
  box.append(el('h2', '', '新建笔记'));

  const typeRow = el('label', 'form-row');
  typeRow.append(el('span', '', '类型'));
  const typeSel = el('select', 'todo-filter');
  const kinds = [['diary', '日记'], ['note', '普通笔记']].concat(
    templates.map((name) => ['tpl:' + name, '模板：' + name])
  );
  for (const [value, label] of kinds) {
    const opt = el('option', '', label);
    opt.value = value;
    typeSel.append(opt);
  }
  typeRow.append(typeSel);

  const titleRow = el('label', 'form-row');
  titleRow.append(el('span', '', '标题'));
  const titleInput = el('input', 'quick-add');
  titleInput.placeholder = '新笔记标题';
  titleRow.append(titleInput);

  const dateRow = el('label', 'form-row');
  dateRow.append(el('span', '', '日期'));
  const dateInput = document.createElement('input');
  dateInput.type = 'date';
  const now = new Date();
  dateInput.value = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
  dateInput.className = 'quick-add';
  dateRow.append(dateInput);

  const syncRows = () => {
    const isDiary = typeSel.value === 'diary';
    dateRow.hidden = !isDiary;
    titleRow.hidden = isDiary;
  };
  typeSel.addEventListener('change', syncRows);

  const actions = el('div', 'modal-actions');
  const cancel = el('button', 'seg', '取消');
  cancel.type = 'button';
  cancel.addEventListener('click', () => mask.remove());
  const ok = el('button', 'seg active', '创建');
  ok.type = 'button';
  ok.addEventListener('click', async () => {
    const value = typeSel.value;
    const payload = {};
    if (value.startsWith('tpl:')) {
      payload.template = value.slice(4);
      payload.type = 'note';
      if (!titleInput.value.trim()) {
        window.alert('请填写标题');
        return;
      }
      payload.title = titleInput.value.trim();
    } else if (value === 'diary') {
      payload.type = 'diary';
      payload.date = dateInput.value || undefined;
    } else {
      payload.type = 'note';
      if (!titleInput.value.trim()) {
        window.alert('请填写标题');
        return;
      }
      payload.title = titleInput.value.trim();
    }
    try {
      const res = await api('/api/notes/new', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      mask.remove();
      await loadSidebar();
      go('/file/' + encodeURIComponent(res.path));
    } catch (err) {
      window.alert('新建失败：' + (err.message || err));
    }
  });
  actions.append(cancel, ok);

  box.append(typeRow, titleRow, dateRow, actions);
  mask.append(box);
  mask.addEventListener('click', (event) => {
    if (event.target === mask) mask.remove();
  });
  document.body.append(mask);
  syncRows();
  titleInput.focus();
}

// 自动化环境（webdriver）不注册，避免无人处理确认框阻塞导航
if (!navigator.webdriver) {
  window.addEventListener('beforeunload', (event) => {
    if (typeof activeDirty === 'function' && activeDirty()) {
      event.preventDefault();
      event.returnValue = '还有未保存的修改，确定离开？';
      return event.returnValue;
    }
    return undefined;
  });
}

const newNoteBtn = document.getElementById('new-note');
if (newNoteBtn) newNoteBtn.addEventListener('click', openNewNoteDialog);

const refreshBtn = document.getElementById('refresh');
if (refreshBtn) {
  refreshBtn.addEventListener('click', async () => {
    refreshBtn.disabled = true;
    try {
      await loadSidebar();
      await route();
    } finally {
      refreshBtn.disabled = false;
    }
  });
}

const SHORTCUTS = [
  ['Ctrl / Cmd + K', '聚焦搜索框'],
  ['/', '聚焦搜索框（不在输入态时）'],
  ['Ctrl / Cmd + S', '立即保存当前笔记'],
  ['Esc', '关闭弹层 / 收起侧边栏'],
  ['?', '打开这个快捷键面板'],
];

function toggleHelp(force) {
  const existing = document.getElementById('help-modal');
  if (existing && force !== true) {
    existing.remove();
    return;
  }
  if (existing) return;
  const mask = el('div', 'modal-mask');
  mask.id = 'help-modal';
  const box = el('div', 'modal');
  box.append(el('h2', '', '快捷键'));
  const list = el('dl', 'shortcut-list');
  for (const [key, desc] of SHORTCUTS) {
    list.append(el('dt', '', key));
    list.append(el('dd', '', desc));
  }
  box.append(list);
  const close = el('button', 'seg', '关闭');
  close.type = 'button';
  close.addEventListener('click', () => mask.remove());
  box.append(close);
  mask.append(box);
  mask.addEventListener('click', (event) => {
    if (event.target === mask) mask.remove();
  });
  document.body.append(mask);
}

document.addEventListener('keydown', (event) => {
  const tag = (event.target.tagName || '').toLowerCase();
  const typing = tag === 'input' || tag === 'textarea' || event.target.isContentEditable;
  if (event.key === '?' && !typing) {
    event.preventDefault();
    toggleHelp();
  }
  if (event.key === 'Escape') {
    const modal = document.getElementById('help-modal');
    if (modal) modal.remove();
  }
});
// 窄屏侧边栏抽屉
const sidebarToggle = document.getElementById('sidebar-toggle');
const backdrop = document.getElementById('backdrop');
const sidebar = document.querySelector('.sidebar');

function closeSidebar() {
  if (sidebar) sidebar.classList.remove('open');
  if (backdrop) backdrop.classList.remove('show');
  if (sidebarToggle) sidebarToggle.setAttribute('aria-expanded', 'false');
}

if (sidebarToggle) {
  sidebarToggle.addEventListener('click', () => {
    const open = sidebar.classList.toggle('open');
    backdrop.classList.toggle('show', open);
    sidebarToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
  });
}
if (backdrop) backdrop.addEventListener('click', closeSidebar);
window.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') closeSidebar();
});
// 点了导航就收起抽屉（窄屏下）
document.addEventListener('click', (event) => {
  if (window.innerWidth > 860) return;
  const target = event.target;
  if (target && (target.closest('.tree .leaf') || target.closest('.tag-node') || target.closest('.quick'))) {
    closeSidebar();
  }
});
async function boot() {
  try {
    try {
      const savedTagSort = localStorage.getItem('nc.tagSort');
      if (savedTagSort) state.tagSort = savedTagSort;
      const savedRange = localStorage.getItem('nc.scheduleRange');
      if (savedRange) state.scheduleRange = savedRange;
    } catch (err) {
      /* ignore */
    }
    const cfg = await api('/api/config').catch(() => null);
    const migration = await api('/api/migration/todo').catch(() => null);
    if (migration && migration.total && localStorage.getItem('nc.migrationDismissed') !== '1') {
      const note = el('div', 'notice');
      note.append(el('span', '', `发现 ${migration.total} 条 todo: 写法的历史待办 `));
      const link = el('a', '', '查看清单');
      link.href = '#/migration';
      link.style.color = 'inherit';
      link.style.textDecoration = 'underline';
      note.append(link);
      note.append(document.createTextNode(' '));
      const dismiss = el('button', 'seg tiny', '知道了');
      dismiss.type = 'button';
      dismiss.addEventListener('click', () => {
        try {
          localStorage.setItem('nc.migrationDismissed', '1');
        } catch (err) {
          /* ignore */
        }
        note.remove();
      });
      note.append(dismiss);
      document.querySelector('.panels').prepend(note);
    }
    // PRD 2.2 / 3.1：打开首页时当天文件不存在则自动新建
    await api('/api/daily', { method: 'POST' }).catch(() => null);
    if (cfg) {
      state.links = !!cfg.links;
      state.duplicateNames = cfg.duplicateNames || [];
      state.largeAssets = cfg.largeAssets || [];
      if (cfg.syncDrive) {
        const note = el('div', 'notice', `目录位于 ${cfg.syncDrive} 内，Git 自动提交可能与同步客户端冲突，建议移到普通目录`);
        document.querySelector('.panels').prepend(note);
      }
      if (state.largeAssets.length) {
        const names = state.largeAssets.map((f) => `${f.path} (${f.sizeMb}MB)`).join('、');
        const note = el('div', 'notice', `assets/ 里有大文件，会让 Git 仓库持续变大：${names}`);
        document.querySelector('.panels').prepend(note);
      }
      if (state.duplicateNames.length) {
        const names = state.duplicateNames.map((d) => d.name).join('、');
        const note = el('div', 'notice', `raw/ 下有同名笔记，裸名暂不解析：${names}`);
        note.title = state.duplicateNames
          .map((d) => `${d.name}: ${d.paths.join(' / ')}`)
          .join('\n');
        document.querySelector('.panels').prepend(note);
      }
    }
    await loadSidebar();
    setGit(await api('/api/git/status').catch(() => null));
  } catch (err) {
    fileTree.append(el('div', 'notice', String(err.message || err)));
  }
  route();
  pollRevision();
}

boot();
