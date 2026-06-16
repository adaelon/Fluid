// U4 validation (B2, deterministic). Exercises the pure command-palette fuzzy
// matcher — fuzzyMatch + fuzzyFilter — with assertions, no browser / DOM.
// Run with: node scripts/fuzzy-check.ts  (Node 24 strips TS types).
//
// The palette UI (input focus, ↑↓ navigation, Enter/click execution) is
// browser-verified (A2); this script locks the matching/ranking those rely on.

import { fuzzyMatch, fuzzyFilter } from '../src/shell/fuzzy.ts'

let failures = 0
function check(label: string, cond: boolean): void {
  if (cond) {
    console.log(`  PASS  ${label}`)
  } else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

console.log('=== fuzzyMatch ===')
check('empty query matches everything with score 0', fuzzyMatch('', 'anything') === 0)
check('subsequence matches', fuzzyMatch('eng', 'engine.py') !== null)
check('non-subsequence returns null', fuzzyMatch('xyz', 'engine.py') === null)
check('case-insensitive', fuzzyMatch('ENG', 'engine.py') !== null)
check('out-of-order returns null', fuzzyMatch('gne', 'eng') === null)
check(
  'segment-start beats mid-word for same chars',
  // "ng" at the start of a path segment vs buried mid-word.
  (fuzzyMatch('ng', 'a/ngram.py') as number) > (fuzzyMatch('ng', 'aengine.py') as number),
)
check(
  'consecutive beats scattered',
  (fuzzyMatch('abc', 'abc') as number) > (fuzzyMatch('abc', 'a_b_c') as number),
)
check(
  'shorter target wins on otherwise-equal match',
  (fuzzyMatch('e', 'e.py') as number) > (fuzzyMatch('e', 'e_longer_name.py') as number),
)

console.log('\n=== fuzzyFilter (rank + stability + limit) ===')
const files = [
  'model_core/alphagpt.py',
  'model_core/engine.py',
  'model_core/factors.py',
  'lord/experiment.py',
  'README.md',
]
const id = (s: string) => s

const newton = fuzzyFilter('alpha', files, id)
check('drops non-matches', newton.length === 1 && newton[0] === 'model_core/alphagpt.py')

const eng = fuzzyFilter('engine', files, id)
check('finds engine.py', eng[0] === 'model_core/engine.py')

const all = fuzzyFilter('', files, id)
check('empty query keeps all in input order', all.length === files.length && all[0] === files[0])

const dotpy = fuzzyFilter('py', files, id)
check('all .py files match "py", .md excluded', dotpy.length === 4 && !dotpy.includes('README.md'))

const limited = fuzzyFilter('', files, id, 2)
check('respects limit', limited.length === 2)

console.log('\n=== command labels ===')
const commands = ['设置 · LLM 后端', '打开文件夹…', '切换追问器', '关闭当前标签页']
const toggle = fuzzyFilter('追问', commands, id)
check('matches Chinese command label', toggle.length === 1 && toggle[0] === '切换追问器')

if (failures > 0) {
  console.error(`\n${failures} check(s) FAILED`)
  process.exit(1)
}
console.log('\nAll fuzzy checks passed.')
