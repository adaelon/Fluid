// S-QREAD-R0 deterministic checks. Run with:
//   node scripts/code-view-check.ts
// Node 24 strips TypeScript annotations; no Vue, DOM or backend is needed.

import { EditorState } from '@codemirror/state'
import { syntaxTree } from '@codemirror/language'
import { EditorView } from '@codemirror/view'
import {
  codeLanguageExtension,
  codeLanguageTagFromPath,
  readOnlyCodeViewExtensions,
} from '../src/codeView.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

const cases = [
  ['src/main.py', 'py'],
  ['src/main.rs', 'rs'],
  ['src/app.ts', 'ts'],
  ['src/app.mts', 'ts'],
  ['src/app.cts', 'ts'],
  ['src/App.tsx', 'tsx'],
  ['src/app.js', 'js'],
  ['src/app.mjs', 'js'],
  ['src/app.cjs', 'js'],
  ['src/App.jsx', 'jsx'],
  ['docs/README.md', 'md'],
  ['docs/guide.markdown', 'md'],
] as const

console.log('=== project path extension -> language tag ===')
for (const [path, expected] of cases) {
  check(`${path} -> ${expected}`, codeLanguageTagFromPath(path) === expected)
}

console.log('\n=== path edge cases stay aligned with backend tags ===')
check('Windows separators are accepted', codeLanguageTagFromPath('src\\main.rs') === 'rs')
check('compound declarations use the last extension', codeLanguageTagFromPath('types/api.d.ts') === 'ts')
check('unknown extensions stay other', codeLanguageTagFromPath('notes.txt') === 'other')
check('extensionless files stay other', codeLanguageTagFromPath('Makefile') === 'other')
check('dotfiles stay other', codeLanguageTagFromPath('.env') === 'other')
check('trailing dots stay other', codeLanguageTagFromPath('src/name.') === 'other')
check('extension matching remains case-sensitive', codeLanguageTagFromPath('src/App.TS') === 'other')

function parsesWithoutError(lang: string, source: string): boolean {
  const state = EditorState.create({ doc: source, extensions: [codeLanguageExtension(lang)] })
  const cursor = syntaxTree(state).cursor()
  do {
    if (cursor.type.isError) return false
  } while (cursor.next())
  return true
}

console.log('\n=== every shared language flavor constructs its parser ===')
check('Python parser accepts Python syntax', parsesWithoutError('py', 'def greet():\n    return "hi"\n'))
check('Rust parser accepts Rust syntax', parsesWithoutError('rs', 'fn main() { println!("hi"); }\n'))
check('TypeScript parser accepts type syntax', parsesWithoutError('ts', 'interface Props { value: string }\n'))
check(
  'TSX parser accepts typed JSX syntax',
  parsesWithoutError('tsx', 'const view: JSX.Element = <main />\n'),
)
check('JavaScript parser accepts JavaScript syntax', parsesWithoutError('js', 'const value = 1\n'))
check('JSX parser accepts JSX syntax', parsesWithoutError('jsx', 'const view = <main />\n'))

console.log('\n=== shared base keeps both read-only guards ===')
const readOnlyState = EditorState.create({
  doc: 'const value = 1\n',
  extensions: readOnlyCodeViewExtensions('ts'),
})
check('state transactions are read-only', readOnlyState.facet(EditorState.readOnly))
check('the CodeMirror content DOM is non-editable', !readOnlyState.facet(EditorView.editable))

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll code-view checks passed.')
