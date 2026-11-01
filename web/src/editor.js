// NotesChain 编辑器：CodeMirror 6 + Markdown，Live Preview L1（隐藏非光标行的语法符号）
import { EditorView, keymap, drawSelection, highlightActiveLine, ViewPlugin, Decoration } from '@codemirror/view';
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
  '.cm-content': { fontFamily: 'inherit', padding: '4px 0', lineHeight: '1.72', caretColor: 'var(--accent)' },
  '.cm-scroller': { fontFamily: 'inherit', overflow: 'auto' },
  '.cm-line': { padding: '0 2px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-activeLine': { backgroundColor: 'rgba(127,127,127,.07)' },
  '.cm-selectionBackground, ::selection': { backgroundColor: 'var(--accent-soft)' },
  '.cm-cursor': { borderLeftColor: 'var(--accent)' },
});

// 光标不在的行，把 Markdown 语法符号（#、**、`、- 等）淡出
const markHide = Decoration.mark({ class: 'cm-syntax-mark' });

const livePreview = ViewPlugin.fromClass(
  class {
    constructor(view) {
      this.decorations = this.build(view);
    }
    update(update) {
      if (update.docChanged || update.viewportChanged || update.selectionSet) {
        this.decorations = this.build(update.view);
      }
    }
    build(view) {
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
            for (const m of text.matchAll(/(\*\*|__|\*|_|`+|~~|^#{1,6}\s|^[-*+]\s|^\d+\.\s)/g)) {
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
  },
  { decorations: (v) => v.decorations }
);

/**
 * 挂载编辑器。
 * @param {{parent: HTMLElement, doc: string, onChange?: (v:string)=>void, onSave?: ()=>void}} opts
 */
export function create(opts) {
  const { parent, doc, onChange, onSave } = opts;
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
        livePreview,
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
