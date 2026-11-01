// NotesChain 编辑器：CodeMirror 6 + Markdown
// L1 Live Preview：非光标行隐藏语法符号
// 额外：待办行渲染为可点复选框；图片行内预览（失败显示占位）
import {
  EditorView,
  keymap,
  drawSelection,
  highlightActiveLine,
  ViewPlugin,
  Decoration,
  WidgetType,
} from '@codemirror/view';
import { EditorState, RangeSetBuilder, StateField } from '@codemirror/state';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { syntaxHighlighting, HighlightStyle } from '@codemirror/language';
import { tags } from '@lezer/highlight';

const highlight = HighlightStyle.define([
  { tag: tags.heading1, fontSize: '1.5em', fontWeight: '700' },
  { tag: tags.heading2, fontSize: '1.24em', fontWeight: '700' },
  { tag: tags.heading3, fontSize: '1.08em', fontWeight: '600' },
  { tag: tags.heading4, fontWeight: '600' },
  { tag: tags.strong, fontWeight: '700' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through' },
  { tag: tags.link, color: 'var(--accent)', textDecoration: 'underline' },
  { tag: tags.url, color: 'var(--muted)' },
  { tag: tags.monospace, fontFamily: 'ui-monospace, Consolas, monospace', color: 'var(--chip-fg)' },
  { tag: tags.quote, color: 'var(--muted)' },
  { tag: tags.list, color: 'var(--muted)' },
  { tag: tags.meta, color: 'var(--faint)' },
  { tag: tags.processingInstruction, color: 'var(--faint)' },
]);

const theme = EditorView.theme({
  '&': { color: 'var(--fg)', backgroundColor: 'transparent', fontSize: '14.5px' },
  '.cm-content': { fontFamily: 'inherit', padding: '4px 0', lineHeight: '1.78', caretColor: 'var(--accent)' },
  '.cm-scroller': { fontFamily: 'inherit', overflow: 'auto' },
  '.cm-line': { padding: '0 2px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-activeLine': { backgroundColor: 'rgba(127,127,127,.07)' },
  '.cm-selectionBackground, ::selection': { backgroundColor: 'var(--accent-soft)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent)' },
});

const markHide = Decoration.mark({ class: 'cm-syntax-mark' });
const tagMark = Decoration.mark({ class: 'cm-tag' });
const timeMark = Decoration.mark({ class: 'cm-time' });

function pad(n) {
  return String(n).padStart(2, '0');
}

function nowStamp() {
  const d = new Date();
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 链接 [文本](地址) */
let linkHandler = null;

function escapeHtml(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}

function inlineHtml(raw) {
  let s = escapeHtml(raw);
  s = s.replace(/`([^`]+)`/g, '<code>$1</code>');
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[^*\w])\*([^*\n]+)\*/g, '$1<em>$2</em>');
  s = s.replace(/(^|[\s(（，。；：])#([\w\u3400-\u9fff/-]+)/g, '$1<span class="tag">#$2</span>');
  return s;
}

class LinkWidget extends WidgetType {
  constructor(text, href) {
    super();
    this.text = text;
    this.href = href;
  }
  eq(other) {
    return other.text === this.text && other.href === this.href;
  }
  toDOM() {
    const a = document.createElement('a');
    a.className = 'cm-link';
    a.textContent = this.text;
    a.title = this.href;
    a.addEventListener('mousedown', (e) => e.preventDefault());
    a.addEventListener('click', (e) => {
      e.preventDefault();
      if (/^https?:/i.test(this.href)) {
        window.open(this.href, '_blank', 'noreferrer');
      } else if (linkHandler) {
        linkHandler(this.href);
      }
    });
    return a;
  }
  ignoreEvent() {
    return false;
  }
}

/** 表格：整块替换为渲染后的 HTML 表格 */
class TableWidget extends WidgetType {
  constructor(html) {
    super();
    this.html = html;
  }
  eq(other) {
    return other.html === this.html;
  }
  toDOM() {
    const wrap = document.createElement('div');
    wrap.className = 'cm-table';
    wrap.contentEditable = 'false';
    wrap.innerHTML = this.html;
    return wrap;
  }
  ignoreEvent() {
    return true;
  }
}

function renderTableHtml(rows) {
  if (!rows.length) return '';
  const cells = (line) =>
    line
      .replace(/^\s*\|/, '')
      .replace(/\|\s*$/, '')
      .split('|')
      .map((c) => c.trim());
  const header = cells(rows[0]);
  const body = rows.slice(2).map(cells);
  let html = '<table><thead><tr>';
  html += header.map((c) => `<th>${inlineHtml(c)}</th>`).join('');
  html += '</tr></thead><tbody>';
  for (const row of body) {
    html += '<tr>' + row.map((c) => `<td>${inlineHtml(c)}</td>`).join('') + '</tr>';
  }
  return html + '</tbody></table>';
}
/** 双链 [[目标]]（PRD 3.5，默认关闭） */
let openLinkHandler = null;

class WikiLinkWidget extends WidgetType {
  constructor(target, missing) {
    super();
    this.target = target;
    this.missing = !!missing;
  }
  eq(other) {
    return other.target === this.target && other.missing === this.missing;
  }
  toDOM() {
    const span = document.createElement('span');
    span.className = this.missing ? 'cm-wikilink missing' : 'cm-wikilink';
    span.textContent = this.target;
    span.title = '[[' + this.target + ']]';
    span.addEventListener('mousedown', (e) => e.preventDefault());
    span.addEventListener('click', (e) => {
      e.preventDefault();
      if (openLinkHandler) openLinkHandler(this.target);
    });
    return span;
  }
  ignoreEvent() {
    return false;
  }
}
/** 待办复选框 */
class CheckboxWidget extends WidgetType {
  constructor(checked, pos) {
    super();
    this.checked = checked;
    this.pos = pos;
  }
  eq(other) {
    return other.checked === this.checked && other.pos === this.pos;
  }
  toDOM(view) {
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.className = 'cm-todo-box';
    box.checked = this.checked;
    box.defaultChecked = this.checked;
    box.addEventListener('mousedown', (e) => e.preventDefault());
    box.addEventListener('click', async (e) => {
      e.preventDefault();
      await toggleTask(view, this.pos);
    });
    return box;
  }
  ignoreEvent() {
    return false;
  }
}

/** 图片预览 */
class ImageWidget extends WidgetType {
  constructor(alt, url) {
    super();
    this.alt = alt || '';
    this.url = url;
  }
  eq(other) {
    return other.url === this.url && other.alt === this.alt;
  }
  toDOM() {
    const wrap = document.createElement('span');
    wrap.className = 'cm-img';
    const img = document.createElement('img');
    img.src = this.url;
    img.alt = this.alt;
    img.loading = 'lazy';
    img.addEventListener('error', () => {
      wrap.classList.add('broken');
      wrap.textContent = '🖼 ' + (this.alt || this.url.replace(/^.*\//, ''));
    });
    wrap.append(img);
    return wrap;
  }
  ignoreEvent() {
    return true;
  }
}

async function toggleTask(view, pos) {
  const line = view.state.doc.lineAt(pos);
  const match = /^(\s*)- \[([ xX])\]\s?(.*)$/.exec(line.text);
  if (!match) return;
  const done = match[2].toLowerCase() === 'x';
  let rest = match[3] || '';
  const markerAt = rest.indexOf('✅');
  if (done && markerAt >= 0) rest = rest.slice(0, markerAt).trimEnd();

  const rawLine = `${match[1]}- [${match[2]}] ${rest}`.trimEnd();
  const nextText = done ? `${match[1]}- [ ] ${rest}` : `${match[1]}- [x] ${rest} ✅ ${nowStamp()}`;
  const before = line.text;

  view.dispatch({ changes: { from: line.from, to: line.to, insert: nextText } });

  if (view.__ncOnToggle) {
    try {
      await view.__ncOnToggle({ line: line.number, rawLine, done: !done });
    } catch (err) {
      // 失败回滚
      const cur = view.state.doc.line(line.number);
      view.dispatch({ changes: { from: cur.from, to: cur.to, insert: before } });
    }
  }
}

function activeLineNumbers(state) {
  const set = new Set();
  for (const range of state.selection.ranges) {
    const first = state.doc.lineAt(range.from).number;
    const last = state.doc.lineAt(range.to).number;
    for (let n = first; n <= last; n += 1) set.add(n);
  }
  return set;
}

function buildTableDecorations(state) {
  const decos = [];
  const doc = state.doc;
  const active = activeLineNumbers(state);
  let n = 1;
  while (n <= doc.lines) {
    const line = doc.line(n);
    if (!line.text.trimStart().startsWith('|') || n >= doc.lines) {
      n += 1;
      continue;
    }
    const sep = doc.line(n + 1).text;
    if (!/^\s*\|?[\s:|-]+\|[\s:|-]*$/.test(sep)) {
      n += 1;
      continue;
    }
    let end = n + 1;
    const rows = [line.text, sep];
    while (end < doc.lines) {
      const next = doc.line(end + 1).text;
      if (!next.trim() || !next.includes('|')) break;
      rows.push(next);
      end += 1;
    }
    let hasActive = false;
    for (let k = n; k <= end; k += 1) if (active.has(k)) hasActive = true;
    if (!hasActive) {
      decos.push(
        Decoration.replace({ widget: new TableWidget(renderTableHtml(rows)), block: true }).range(
          doc.line(n).from,
          doc.line(end).to
        )
      );
    }
    n = end + 1;
  }
  return Decoration.set(decos, true);
}

const tableField = StateField.define({
  create: (state) => buildTableDecorations(state),
  update: (deco, tr) => (tr.docChanged || tr.selection ? buildTableDecorations(tr.state) : deco),
  provide: (field) => EditorView.decorations.from(field),
});
function buildDecorations(view, assetFor, linksEnabled, linkExists) {
  const builder = new RangeSetBuilder();
  const activeLines = new Set();
  for (const range of view.state.selection.ranges) {
    const first = view.state.doc.lineAt(range.from).number;
    const last = view.state.doc.lineAt(range.to).number;
    for (let n = first; n <= last; n++) activeLines.add(n);
  }
  const doc = view.state.doc;

  for (const { from, to } of view.visibleRanges) {
    let pos = from;
    while (pos <= to) {
      const line = doc.lineAt(pos);
      const text = line.text;

      if (!activeLines.has(line.number)) {
        const ranges = [];

        const task = /^(\s*)- \[([ xX])\]\s?/.exec(text);
        if (task) {
          ranges.push({
            from: line.from,
            to: line.from + task[0].length,
            deco: Decoration.replace({
              widget: new CheckboxWidget(task[2].toLowerCase() === 'x', line.from + task[1].length),
            }),
          });
          const marker = text.indexOf('✅');
          if (marker >= 0) {
            ranges.push({ from: line.from + marker, to: line.to, deco: markHide });
          }
        }

        for (const m of text.matchAll(/!\[([^\]]*)\]\(([^)\s]+)\)/g)) {
          const startAt = line.from + m.index;
          ranges.push({
            from: startAt,
            to: startAt + m[0].length,
            deco: Decoration.replace({ widget: new ImageWidget(m[1], assetFor(m[2])) }),
          });
        }

        for (const m of text.matchAll(/(?<!!)\[([^\]]+)\]\(([^)\s]+)\)/g)) {
          const startAt = line.from + m.index;
          ranges.push({
            from: startAt,
            to: startAt + m[0].length,
            deco: Decoration.replace({ widget: new LinkWidget(m[1], m[2]) }),
          });
        }

        if (linksEnabled) {
          for (const m of text.matchAll(/\[\[([^\]\n]+)\]\]/g)) {
            const startAt = line.from + m.index;
            ranges.push({
              from: startAt,
              to: startAt + m[0].length,
              deco: Decoration.replace({
                widget: new WikiLinkWidget(m[1].trim(), linkExists ? !linkExists(m[1].trim()) : false),
              }),
            });
          }
        }

        // 日程时间 HH:mm[-HH:mm] 加粗（PRD 5.2 L1）
        for (const m of text.matchAll(/(^|\s)(\d{1,2}:\d{2}(?:-\d{1,2}:\d{2})?)/g)) {
          const startAt = line.from + m.index + m[1].length;
          ranges.push({ from: startAt, to: startAt + m[2].length, deco: timeMark });
        }

        for (const m of text.matchAll(/(^|[\s(（，。；：])#([\w\u3400-\u9fff/-]+)/g)) {
          const startAt = line.from + m.index + m[1].length;
          ranges.push({ from: startAt, to: startAt + 1 + m[2].length, deco: tagMark });
        }

        for (const m of text.matchAll(/(\*\*|__|\*|_|`+|~~|^#{1,6}\s)/g)) {
          const startAt = line.from + m.index;
          ranges.push({ from: startAt, to: startAt + m[0].length, deco: markHide });
        }

        ranges.sort((a, b) => a.from - b.from || a.to - b.to);
        let lastTo = -1;
        for (const r of ranges) {
          if (r.from < lastTo) continue; // 跳过重叠，保证 RangeSet 有序不交叉
          builder.add(r.from, r.to, r.deco);
          lastTo = r.to;
        }
      }

      pos = line.to + 1;
      if (line.to >= to) break;
    }
  }
  return builder.finish();
}
function livePreviewExtension(assetFor, onToggleTodo, linksEnabled, linkExists) {
  class LivePreviewPlugin {
    constructor(view) {
      this.view = view;
      view.__ncOnToggle = onToggleTodo || null;
      this.decorations = buildDecorations(view, assetFor, linksEnabled, linkExists);
    }
    update(update) {
      if (update.docChanged || update.viewportChanged || update.selectionSet) {
        this.decorations = buildDecorations(update.view, assetFor, linksEnabled, linkExists);
      }
    }
  }
  return ViewPlugin.fromClass(LivePreviewPlugin, { decorations: (v) => v.decorations });
}

/**
 * @param {{parent: HTMLElement, doc: string, onChange?: Function, onSave?: Function, onToggleTodo?: Function, assetFor?: (path:string)=>string}} opts
 */
export function create(opts) {
  const { parent, doc, onChange, onSave, onToggleTodo, assetFor, links, onOpenLink } = opts;
  openLinkHandler = onOpenLink || null;
  linkHandler = opts.onOpenHref || null;
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: doc || '',
      extensions: [
        history(),
        drawSelection(),
        highlightActiveLine(),
        EditorView.lineWrapping,
        markdown({ base: markdownLanguage, addKeymap: true }),
        syntaxHighlighting(highlight),
        livePreviewExtension(assetFor || ((p) => p), onToggleTodo, !!links, opts.linkExists || null),
        tableField,
        theme,
        keymap.of([
          {
            key: 'Mod-s',
            preventDefault: true,
            run: () => {
              if (onSave) onSave();
              return true;
            },
          },
          ...defaultKeymap,
          ...historyKeymap,
          indentWithTab,
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && onChange) onChange(update.state.doc.toString());
        }),
      ],
    }),
  });


  return {
    view,
    getValue: () => view.state.doc.toString(),
    setValue: (text) => {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
    },
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}
