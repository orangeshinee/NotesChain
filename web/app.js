/* NotesChain 前端：侧边栏（文件 / 标签）、Markdown 渲染、搜索、首页待办 */

const state = {
  files: null,
  tags: null,
  activePath: null,
  activeTag: null,
  searchStatus: 'all',
  scheduleRange: 'week',
  scheduleData: null,
  scheduleTimer: null,
  links: false,
  todoLimit: 1000,
  todoStatus: 'open',
  duplicateNames: [],
  pathSet: new Set(),
  tagSort: 'name',
  todoTag: '',
  searchScope: 'all',
  largeAssets: [],
  collapsedTags: new Set(),
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
  s = s.replace(/\u0001(\d+)\u0001/g, (_, i) => `<code>${esc(codes[Number(i)])}</code>`);
  return s;
}

function renderMarkdown(text) {
  const lines = text.split('\n');
  const n = lines.length;
  const out = [];
  let i = 0;

  while (i < n) {
    const line = lines[i];
    const start = i + 1;

    // 围栏代码块
    const fence = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)\s*$/.exec(line);
    if (fence) {
      const marker = fence[1];
      const ch = marker[0] === '`' ? '`' : '~';
      const len = marker.length;
      const lang = fence[2] || '';
      const buf = [];
      i++;
      const closeRe = new RegExp('^\\s{0,3}' + ch + '{' + len + ',}\\s*$');
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
      const list = parseList(lines, i);
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

  return out.join('\n');
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

function parseList(lines, start) {
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
      line: i + 1,
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
      inner = `<span class="box">${done ? '☑' : '☐'}</span>${inline(task[2])}`;
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
async function showHome() {
  state.activePath = null;
  state.activeTag = null;
  markActive();
  main.replaceChildren(docHead('首页', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState());
  main.append(wrap);
  try {
    const [todos, git, schedule] = await Promise.all([
      api('/api/todos?status=' + state.todoStatus + '&limit=' + state.todoLimit + (state.todoTag ? '&tag=' + encodeURIComponent(state.todoTag) : '')),
      api('/api/git/status').catch(() => null),
      api('/api/schedule?range=' + state.scheduleRange).catch(() => null),
    ]);
    setGit(git);
    state.scheduleData = schedule;
    startScheduleTicker();
    const parts = [];
    if (schedule) parts.push(buildSchedule(schedule));
    parts.push(buildTodoSection(todos, () => { state.todoLimit = 50000; showHome(); }));
    wrap.replaceChildren(...parts);
    main.replaceChildren(docHead('首页', `未完成 ${todos.total} 条`), wrap);
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
  section.append(head);

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
      decorated.sort((a, b) => {
        if (a.status === 'live' && b.status !== 'live') return -1;
        if (b.status === 'live' && a.status !== 'live') return 1;
        return String(a.item.start).localeCompare(String(b.item.start)) || a.order - b.order;
      });

      const list = el('ul', 'slots');
      for (const entry of decorated) {
        const item = entry.item;
        const li = el('li', entry.status);
        const time = item.end ? `${item.start}–${item.end}` : item.start;
        li.append(el('span', 'time', time));
        if (entry.status === 'live') li.append(el('span', 'live-pill', '进行中'));
        li.append(el('span', 'what', item.title));
        if (item.place) li.append(el('span', 'place', '@' + item.place));
        for (const tag of item.tags) li.append(el('span', 'tag', '#' + tag));
        list.append(li);
      }
      box.append(list);
    } else {
      box.append(el('div', 'muted empty-line', '无日程'));
    }

    if (day.today && day.suspected && day.suspected.length) {
      box.append(
        el('div', 'unrec muted', `有 ${day.suspected.length} 行像日程但不在日程段内（第 ${day.suspected.join('、')} 行）`)
      );
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
    showHome();
  });
  head.append(filter);

  const toggle = el('span', 'segmented');
  for (const [value, label] of [['open', '未完成'], ['done', '已完成'], ['all', '全部']]) {
    const btn = el('button', state.todoStatus === value ? 'seg active' : 'seg', label);
    btn.type = 'button';
    btn.addEventListener('click', () => {
      state.todoStatus = value;
      showHome();
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
    const raw = localStorage.getItem('nc.todoCols');
    const saved = raw ? JSON.parse(raw) : null;
    return Array.isArray(saved) && saved.length === 4 ? saved : null;
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
        localStorage.setItem('nc.todoCols', JSON.stringify(colEls.map((c) => c.style.width)));
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
function buildTodoTable(items) {
  const table = el('table', 'todos');
  const saved = loadColWidths();
  const defaults = ['38px', '34%', '22%', 'auto'];
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
  ['', '事项', '文件', '标签'].forEach((label, index) => {
    const th = el('th');
    th.append(el('span', '', label));
    if (index > 0 && index < 3) th.append(makeColResizer(index, colEls));
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

  const makeRow = (todo) => {
    const tr = el('tr');
    tr.dataset.path = todo.path;
    tr.dataset.line = todo.line;
    tr.dataset.rawLine = todo.raw_line;
    tr.dataset.version = todo.version;

    const tdCheck = el('td', 'check');
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = !!todo.done;
    box.addEventListener('change', () => toggleTodo(tr, box));
    tdCheck.append(box);
    if (todo.done) tr.classList.add('is-done');

    const tdText = el('td');
    if (todo.indent) tdText.style.paddingLeft = `${10 + todo.indent * 8}px`;
    const text = el('span', 'text', (todo.indent ? '· ' : '') + todo.text);
    tdText.append(text);
    if (todo.done_at) tdText.append(el('span', 'done-at', '✅ ' + todo.done_at));

    const tdFile = el('td');
    const file = el('span', 'file', todo.path);
    file.append(el('span', 'ln', ':' + todo.line));
    file.addEventListener('click', () => go('/file/' + encodeURIComponent(todo.path)));
    tdFile.append(file);

    const tdTags = el('td');
    if (todo.tags.length) {
      for (const tag of todo.tags) {
        const chip = el('span', 'tag', '#' + tag);
        chip.addEventListener('click', () => go('/tag/' + encodeURIComponent(tag)));
        tdTags.append(chip);
      }
    } else {
      tdTags.append(el('span', 'no-tag', '—'));
    }

    tr.append(tdCheck, tdText, tdFile, tdTags);
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
    for (let i = rendered; i < end; i += 1) frag.append(makeRow(items[i]));
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
    tr.querySelector('td:nth-child(2)').append(el('div', 'notice', String(err.message || err)));
  }
}

async function showFile(path, line) {
  state.activePath = path;
  state.activeTag = null;
  markActive();
  teardownEditor();
  main.replaceChildren(loadingState('打开 ' + path + ' …'));

  let data;
  try {
    data = await api('/api/files/' + encodeURI(path));
  } catch (err) {
    main.replaceChildren(docHead(path.split('/').pop(), '', path), errorState(err));
    return;
  }

  let version = data.version;
  let eol = data.eol || 'lf';
  let dirty = false;

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

  // 注意：预览保持只读。曾尝试在预览里勾选待办，成功后会顺手把隐藏中的编辑器 setValue，
  // 实测 CodeMirror 在 display:none 容器里会陷入测量死循环、整页卡死 —— 已回退。
  const previewBtn = el('button', 'seg', '预览');
  previewBtn.type = 'button';
  previewBtn.title = '在渲染视图与编辑视图之间切换';
  previewBtn.addEventListener('click', () => {
    state.previewMode = !state.previewMode;
    if (state.previewMode) {
      article.replaceChildren(...renderPreviewNodes(currentEditor ? currentEditor.getValue() : data.content, path));
      article.hidden = false;
      host.hidden = true;
      previewBtn.textContent = '编辑';
    } else {
      article.hidden = true;
      host.hidden = false;
      previewBtn.textContent = '预览';
    }
  });
  head.append(previewBtn);

  main.replaceChildren(head, layout);
  renderOutline(outline, data.content);

  const flush = async (force = false) => {
    if (!currentEditor || !dirty) return;
    const content = currentEditor.getValue();
    setStatus(status, 'saving', '保存中…');
    try {
      const res = await api('/api/files/' + encodeURI(path) + (force ? '?force=true' : ''), {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content, version, eol }),
      });
      version = res.version;
      dirty = false;
      setStatus(status, 'saved', '已保存 ' + timeNow());
      hideConflict(wrap);
    } catch (err) {
      if (err.code === 'conflict') {
        setStatus(status, 'conflict', '有冲突');
        showConflict(wrap, {
          onReload: async () => {
            try {
              const fresh = await api('/api/files/' + encodeURI(path));
              version = fresh.version;
              currentEditor.setValue(fresh.content);
              dirty = false;
              hideConflict(wrap);
              setStatus(status, 'saved', '已重新加载');
            } catch (e) {
              setStatus(status, 'conflict', '重新加载失败');
            }
          },
          onOverwrite: () => flush(true),
        });
      } else {
        setStatus(status, 'conflict', '保存失败');
        showConflict(wrap, { message: String(err.message || err) });
      }
    }
  };

  activeFlush = () => flush(false);
  activeDirty = () => dirty;

  const scheduleSave = () => {

    dirty = true;
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
      flush(false);
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
      // 编辑器里点待办复选框：直接按行回写，不走整文件保存
      const res = await api('/api/todos', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path, line: info.line, raw_line: info.rawLine, version, done: info.done }),
      });
      version = res.version;
      dirty = false;
      clearTimeout(saveTimer);
      setStatus(status, 'saved', '已保存 ' + timeNow());
    },
  });
  currentEditor.focus();
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
      const resolved = item.resolved ? item.resolved + '.md' : '未命中';
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

function renderPreviewNodes(content, filePath) {
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
  state.activePath = null;
  markActive();
  main.replaceChildren(docHead('搜索', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState(`搜索「${q}」…`));
  main.append(wrap);
  try {
    const data = await api('/api/search?q=' + encodeURIComponent(q) + '&status=' + state.searchStatus + '&scope=' + state.searchScope);
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      const meta = el('div', 'meta', `${item.path}:${item.line}`);
      meta.append(el('span', 'kind', kindLabel(item.kind)));
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
      li.append(el('div', 'meta', item.path));
      li.append(el('div', 'snippet', item.snippet));
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path)));
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
      const now = new Date();
      if (
        year.year === String(now.getFullYear()) &&
        month.month === String(now.getMonth() + 1).padStart(2, '0')
      ) {
        monthBox.open = true;
      }
      for (const entry of month.entries) {
        monthBox.append(fileLeaf(entry.path, entry.path.split('/').pop().replace(/\.md$/, ''), entry.todos));
        total++;
      }
      yearBox.append(monthBox);
    }
    if (year.year === new Date().getFullYear().toString()) yearBox.open = true;
    fileTree.append(yearBox);
  }

  if (notes && notes.entries.length) {
    const box = el('details');
    const sum = el('summary');
    sum.append(el('span', '', 'raw / 其他'));
    sum.append(el('span', 'n', String(notes.entries.length)));
    box.append(sum);
    for (const entry of notes.entries) {
      box.append(fileLeaf(entry.path, entry.path.replace(/\.md$/, ''), entry.todos));
      total++;
    }
    box.open = true;
    fileTree.append(box);
  }

  if (!total) fileTree.append(el('div', 'empty-state', '还没有 Markdown 文件。'));
  document.getElementById('file-count').textContent = String(total);
}

function fileLeaf(path, label, todos) {
  const leaf = el('a', 'leaf', label);
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
        const collapsed = state.collapsedTags.has(node.path);
        caret.classList.toggle('open', !collapsed);
        caret.addEventListener('click', (event) => {
          event.stopPropagation();
          const sub = row.nextElementSibling;
          if (!sub) return;
          if (state.collapsedTags.has(node.path)) state.collapsedTags.delete(node.path);
          else state.collapsedTags.add(node.path);
          sub.hidden = !sub.hidden;
          caret.classList.toggle('open', !sub.hidden);
        });
      }

      row.append(caret);
      row.append(el('span', 'tag-name', node.name));
      row.append(el('span', 'n', `${node.notes} 篇 / ${node.todos} 待办`));
      row.addEventListener('click', () => go('/tag/' + encodeURIComponent(node.path)));
      container.append(row);

      if (hasChildren) {
        const sub = el('div', 'tag-children');
        sub.hidden = state.collapsedTags.has(node.path);
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
function markActive() {
  for (const [path, leaf] of state.leafEls) leaf.classList.toggle('active', path === state.activePath);
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
    const flush = activeFlush;
    activeFlush = null;
    activeDirty = null;
    await flush();
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

function openHref(href) {
  const clean = String(href || '').split('#')[0].trim();
  if (!clean) return;
  const base = state.activePath ? state.activePath.split('/').slice(0, -1).join('/') : '';
  const segs = [];
  for (const seg of ((base ? base + '/' : '') + clean).split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') segs.pop();
    else segs.push(seg);
  }
  const path = segs.join('/');
  if (!/\.md$/i.test(path)) return;
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
      await api('/api/files/' + encodeURI(path) + '?force=true', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content: '', version: '' }),
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
    await showHome();
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
