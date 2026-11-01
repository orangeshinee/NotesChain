/* NotesChain 前端：侧边栏（文件 / 标签）、Markdown 渲染、搜索、首页待办 */

const state = {
  files: null,
  tags: null,
  activePath: null,
  activeTag: null,
  searchStatus: 'all',
  scheduleRange: 'week',
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
      api('/api/todos'),
      api('/api/git/status').catch(() => null),
      api('/api/schedule?range=' + state.scheduleRange).catch(() => null),
    ]);
    setGit(git);
    const parts = [];
    if (schedule) parts.push(buildSchedule(schedule));
    parts.push(buildTodoSection(todos));
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
      showHome();
    });
    toggle.append(btn);
  }
  head.append(toggle);
  section.append(head);

  const days = data.days || [];
  if (!days.length) {
    section.append(el('div', 'empty-state', '这一段时间里没有日记文件。'));
    return section;
  }

  for (const day of days) {
    const box = el('div', day.today ? 'day today' : 'day');
    const dh = el('div', 'day-head');
    if (day.today) dh.append(el('span', 'pill', '今天'));
    dh.append(el('span', 'date', `${day.date.slice(5).replace('-', '/')} ${day.weekday}`));
    if (day.items.length) dh.append(el('span', 'muted', `${day.items.length} 项`));
    box.append(dh);

    if (day.items.length) {
      const list = el('ul', 'slots');
      for (const item of day.items) {
        const li = el('li');
        const time = item.end ? `${item.start}–${item.end}` : item.start;
        li.append(el('span', 'time', time));
        li.append(el('span', 'what', item.title));
        if (item.place) li.append(el('span', 'place', '@' + item.place));
        for (const tag of item.tags) li.append(el('span', 'tag', '#' + tag));
        list.append(li);
      }
      box.append(list);
    } else {
      box.append(el('div', 'muted empty-line', '无日程'));
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

function buildTodoSection(todos) {
  const section = el('section', 'todos-section');
  const head = el('div', 'section-head');
  head.append(el('h2', '', '待办'));
  head.append(el('span', 'muted', `${todos.total} 条未完成`));
  section.append(head, buildTodoTable(todos.items));
  return section;
}

function buildTodoTable(items) {
  const table = el('table', 'todos');
  const head = el('thead');
  const hr = el('tr');
  ['', '事项', '文件', '标签'].forEach((h) => hr.append(el('th', '', h)));
  head.append(hr);
  const body = el('tbody');

  if (!items.length) {
    const tr = el('tr');
    const td = el('td', '', '没有未完成的待办，清静。');
    td.colSpan = 4;
    tr.append(td);
    body.append(tr);
  }

  for (const todo of items) {
    const tr = el('tr');
    tr.dataset.path = todo.path;
    tr.dataset.line = todo.line;
    tr.dataset.rawLine = todo.raw_line;
    tr.dataset.version = todo.version;

    const tdCheck = el('td', 'check');
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.addEventListener('change', () => toggleTodo(tr, box));
    tdCheck.append(box);

    const tdText = el('td');
    if (todo.indent) tdText.style.paddingLeft = `${10 + todo.indent * 8}px`;
    const text = el('span', 'text', (todo.indent ? '· ' : '') + todo.text);
    tdText.append(text);

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
    body.append(tr);
  }

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
  let dirty = false;

  const name = path.split('/').pop();
  const head = docHead(name, '', path);
  const status = el('span', 'status saved', '已保存');
  head.append(status);
  const wrap = el('div', 'wrap');
  const host = el('div', 'editor-host');
  wrap.append(host);
  main.replaceChildren(head, wrap);

  const flush = async (force = false) => {
    if (!currentEditor || !dirty) return;
    const content = currentEditor.getValue();
    setStatus(status, 'saving', '保存中…');
    try {
      const res = await api('/api/files/' + encodeURI(path) + (force ? '?force=true' : ''), {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content, version }),
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

  currentEditor = makeEditor(host, data.content, scheduleSave, () => {
    clearTimeout(saveTimer);
    flush(false);
  });
  currentEditor.focus();
  if (line) scrollToLine(line);
}

function makeEditor(host, content, onChange, onSave) {
  if (window.NotesChainEditor && typeof window.NotesChainEditor.create === 'function') {
    return window.NotesChainEditor.create({ parent: host, doc: content, onChange, onSave });
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

function scrollToLine(line) {
  const blocks = Array.from(document.querySelectorAll('.doc [data-line]'));
  let target = null;
  for (const block of blocks) {
    const ln = Number(block.dataset.line);
    if (Number.isFinite(ln) && ln <= line) target = block;
    else if (target) break;
  }
  if (target) {
    target.scrollIntoView({ block: 'center', behavior: 'smooth' });
    target.classList.add('ln-highlight');
    setTimeout(() => target.classList.remove('ln-highlight'), 2200);
  }
}

async function showSearch(q) {
  state.activePath = null;
  markActive();
  main.replaceChildren(docHead('搜索', ''));
  const wrap = el('div', 'wrap');
  wrap.append(loadingState(`搜索「${q}」…`));
  main.append(wrap);
  try {
    const data = await api('/api/search?q=' + encodeURIComponent(q) + '&status=' + state.searchStatus);
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      const meta = el('div', 'meta', `${item.path}:${item.line}`);
      meta.append(el('span', 'kind', kindLabel(item.kind)));
      const snippet = el('div', 'snippet');
      snippet.innerHTML = highlight(item.snippet, q);
      li.append(meta, snippet);
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path) + (item.line ? '@' + item.line : '')));
      list.append(li);
    }
    if (!data.items.length) {
      wrap.replaceChildren(el('div', 'empty-state', `没有命中「${q}」的内容。`));
    } else {
      wrap.replaceChildren(list);
    }
    const head = docHead('搜索', `${data.total} 处命中`, q);
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
    const data = await api('/api/search?tag=' + encodeURIComponent(path));
    const list = el('ul', 'results');
    for (const item of data.items) {
      const li = el('li');
      li.append(el('div', 'meta', item.path));
      li.append(el('div', 'snippet', item.snippet));
      li.addEventListener('click', () => go('/file/' + encodeURIComponent(item.path)));
      list.append(li);
    }
    if (!data.items.length) wrap.replaceChildren(el('div', 'empty-state', '这个标签下还没有笔记。'));
    else wrap.replaceChildren(list);
    main.replaceChildren(docHead('#' + path, `${data.total} 篇`, ''), wrap);
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
        monthBox.append(fileLeaf(entry.path, entry.path.split('/').pop().replace(/\.md$/, '')));
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
      box.append(fileLeaf(entry.path, entry.path.replace(/\.md$/, '')));
      total++;
    }
    box.open = true;
    fileTree.append(box);
  }

  if (!total) fileTree.append(el('div', 'empty-state', '还没有 Markdown 文件。'));
  document.getElementById('file-count').textContent = String(total);
}

function fileLeaf(path, label) {
  const leaf = el('a', 'leaf', label);
  leaf.title = path;
  leaf.addEventListener('click', () => go('/file/' + encodeURIComponent(path)));
  state.leafEls.set(path, leaf);
  return leaf;
}

function renderTagTree(nodes) {
  tagTree.replaceChildren();
  if (!nodes || !nodes.length) {
    tagTree.append(el('div', 'empty-state', '还没有标签。'));
    return;
  }
  const walk = (list, depth, container) => {
    for (const node of list) {
      const row = el('div', 'tag-node', node.name);
      row.style.paddingLeft = `${8 + depth * 12}px`;
      row.title = '#' + node.path;
      row.append(el('span', 'n', `${node.notes} 篇 / ${node.todos} 待办`));
      row.dataset.tag = node.path;
      row.addEventListener('click', () => go('/tag/' + encodeURIComponent(node.path)));
      container.append(row);
      if (node.children && node.children.length) walk(node.children, depth + 1, container);
    }
  };
  walk(nodes, 0, tagTree);
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
  box.textContent = git.available
    ? git.last_commit
      ? '上次提交 ' + git.last_commit.slice(0, 16).replace('T', ' ')
      : '尚无提交'
    : '未检测到 git';
}

// ---------------------------------------------------------------- 路由

async function route() {
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
  return showHome();
}

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

async function boot() {
  try {
    await loadSidebar();
    setGit(await api('/api/git/status').catch(() => null));
  } catch (err) {
    fileTree.append(el('div', 'notice', String(err.message || err)));
  }
  route();
}

boot();
