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
import { EditorState, RangeSetBuilder } from '@codemirror/state';
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

function pad(n) {
  return String(n).padStart(2, '0');
}

function nowStamp() {
  const d = new Date();
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 双链 [[目标]]（PRD 3.5，默认关闭） */
let openLinkHandler = null;

class WikiLinkWidget extends WidgetType {
  constructor(target) {
    super();
    this.target = target;
  }
  eq(other) {
    return other.target === this.target;
  }
  toDOM() {
    const span = document.createElement('span');
    span.className = 'cm-wikilink';
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

function buildDecorations(view, assetFor, linksEnabled) {
  const builder = new RangeSetBuilder();
  const activeLines = new Set();
  for (const range of view.state.selection.ranges) {
    const first = view.state.doc.lineAt(range.from).number;
    const last = view.state.doc.lineAt(range.to).number;
    for (let n = first; n <= last; n++) activeLines.add(n);
  }

  for (const { from, to } of view.visibleRanges) {
    let pos = from;
    while (pos <= to) {
      const line = view.state.doc.lineAt(pos);
      if (!activeLines.has(line.number)) {
        const text = line.text;

        // 待办复选框：替换 `- [ ]` 标记
        const task = /^(\s*)- \[([ xX])\]\s?/.exec(text);
        if (task) {
          builder.add(
            line.from,
            line.from + task[0].length,
            Decoration.replace({
              widget: new CheckboxWidget(task[2].toLowerCase() === 'x', line.from + task[1].length),
            })
          );
          // 已完成行的 ✅ 时间戳淡出
          const marker = text.indexOf('✅');
          if (marker >= 0) {
            builder.add(line.from + marker, line.to, markHide);
          }
        }

        // 图片 → 行内预览
        for (const m of text.matchAll(/!\[([^\]]*)\]\(([^)\s]+)\)/g)) {
          const start = line.from + m.index;
          builder.add(
            start,
            start + m[0].length,
            Decoration.replace({ widget: new ImageWidget(m[1], assetFor(m[2])) })
          );
        }

        // 双链
        if (linksEnabled) {
          for (const m of text.matchAll(/\[\[([^\]\n]+)\]\]/g)) {
            const start = line.from + m.index;
            builder.add(start, start + m[0].length, Decoration.replace({ widget: new WikiLinkWidget(m[1].trim()) }));
          }
        }

        // 其余语法符号淡出
        for (const m of text.matchAll(/(\*\*|__|\*|_|`+|~~|^#{1,6}\s)/g)) {
          const start = line.from + m.index;
          builder.add(start, start + m[0].length, markHide);
        }
      }
      pos = line.to + 1;
      if (line.to >= to) break;
    }
  }
  return builder.finish();
}

function livePreviewExtension(assetFor, onToggleTodo, linksEnabled) {
  class LivePreviewPlugin {
    constructor(view) {
      this.view = view;
      view.__ncOnToggle = onToggleTodo || null;
      this.decorations = buildDecorations(view, assetFor, linksEnabled);
    }
    update(update) {
      if (update.docChanged || update.viewportChanged || update.selectionSet) {
        this.decorations = buildDecorations(update.view, assetFor, linksEnabled);
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
        livePreviewExtension(assetFor || ((p) => p), onToggleTodo, !!links),
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
