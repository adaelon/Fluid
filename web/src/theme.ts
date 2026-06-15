// Fluid dark theme for CodeMirror 6 (ADR-0016, U-R1).
//
// Before this, Editor.vue shipped only `basicSetup`, whose default highlight
// style is tuned for light backgrounds — rendered inside the #0d1117 GitHub-dark
// shell it read as harsh light syntax on dark. This theme + HighlightStyle match
// the existing GitHub-dark palette (same vars as styles.css :root) so the code
// reading area belongs to the shell.

import { EditorView } from '@codemirror/view'
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { tags as t } from '@lezer/highlight'
import type { Extension } from '@codemirror/state'

const BG = '#0d1117'
const FG = '#c9d1d9'
const MUTED = '#8b949e'
const MONO = "'JetBrains Mono', 'SFMono-Regular', ui-monospace, Consolas, monospace"

const editorChrome = EditorView.theme(
  {
    '&': { color: FG, backgroundColor: BG },
    '.cm-scroller': { fontFamily: MONO, lineHeight: '1.6' },
    '.cm-content': { caretColor: FG },
    '.cm-gutters': { backgroundColor: BG, color: '#484f58', border: 'none' },
    '.cm-lineNumbers .cm-gutterElement': { padding: '0 10px 0 8px' },
    '.cm-activeLine': { backgroundColor: 'rgba(177, 186, 196, 0.05)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: MUTED },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
      backgroundColor: 'rgba(56, 139, 253, 0.25)',
    },
  },
  { dark: true },
)

// GitHub-dark syntax palette.
const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.controlKeyword, t.modifier], color: '#ff7b72' },
  { tag: [t.string, t.special(t.string), t.regexp], color: '#a5d6ff' },
  { tag: [t.comment, t.lineComment, t.blockComment], color: MUTED, fontStyle: 'italic' },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: '#d2a8ff' },
  { tag: [t.number, t.bool, t.atom, t.constant(t.variableName)], color: '#79c0ff' },
  { tag: [t.typeName, t.className, t.namespace], color: '#ffa657' },
  { tag: [t.definition(t.variableName), t.variableName, t.propertyName], color: FG },
  { tag: [t.operator, t.punctuation, t.separator, t.bracket], color: FG },
  { tag: [t.meta, t.annotation], color: MUTED },
  { tag: [t.invalid], color: '#ffa198' },
])

export const fluidDarkTheme: Extension = [editorChrome, syntaxHighlighting(highlight)]
