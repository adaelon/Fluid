import { EditorState, type Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { javascript } from '@codemirror/lang-javascript'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'
import { basicSetup } from 'codemirror'
import { fluidDarkTheme } from './theme.ts'

export type CodeLanguageTag = 'py' | 'rs' | 'ts' | 'tsx' | 'js' | 'jsx' | 'md' | 'other'

const LANGUAGE_TAG_BY_EXTENSION: Readonly<Record<string, CodeLanguageTag>> = {
  py: 'py',
  rs: 'rs',
  ts: 'ts',
  mts: 'ts',
  cts: 'ts',
  tsx: 'tsx',
  js: 'js',
  mjs: 'js',
  cjs: 'js',
  jsx: 'jsx',
  md: 'md',
  markdown: 'md',
}

/** Mirror the backend project-tree tags for code views that only receive a path. */
export function codeLanguageTagFromPath(path: string): CodeLanguageTag {
  const basenameStart = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1
  const extensionStart = path.lastIndexOf('.')
  if (extensionStart <= basenameStart || extensionStart === path.length - 1) return 'other'
  return LANGUAGE_TAG_BY_EXTENSION[path.slice(extensionStart + 1)] ?? 'other'
}

export function codeLanguageExtension(lang: string): Extension {
  if (lang === 'py') return python()
  if (lang === 'rs') return rust()
  // TS/JS family shares one parser package; the backend tag selects its flavor.
  if (lang === 'ts') return javascript({ typescript: true })
  if (lang === 'tsx') return javascript({ typescript: true, jsx: true })
  if (lang === 'js') return javascript()
  if (lang === 'jsx') return javascript({ jsx: true })
  return []
}

/**
 * Shared CodeMirror base for source-reading surfaces. `beforeLanguage` lets the
 * main Editor retain its font compartment in the original extension order.
 * `basicSetup` supplies line numbers; both state and DOM editing are disabled.
 */
export function readOnlyCodeViewExtensions(
  lang: string,
  beforeLanguage: readonly Extension[] = [],
): Extension[] {
  return [
    basicSetup,
    fluidDarkTheme,
    ...beforeLanguage,
    codeLanguageExtension(lang),
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
  ]
}
