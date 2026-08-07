// S-FIND-0/S-FIND-CM1 deterministic checks. Run with:
//   node scripts/in-file-find-check.ts
// Node 24 strips TypeScript annotations; no Vue, DOM or backend is needed.

import { EditorState, Text as CmText, Transaction } from '@codemirror/state'
import { getSearchQuery, search, setSearchQuery } from '@codemirror/search'
import {
  collectInFileMatches,
  createInFileSearchQuery,
  currentInFileMatch,
  moveInFileFindCurrent,
  snapshotInFileFindState,
  type FindRange,
  type InFileFindQuery,
} from '../src/inFileFind.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function sameRanges(actual: readonly FindRange[], expected: readonly FindRange[]): boolean {
  return JSON.stringify(actual) === JSON.stringify(expected)
}

function cmText(source: string): CmText {
  return CmText.of(source.split('\n'))
}

function rangesFromBoth(
  source: string,
  query: InFileFindQuery,
): { state: FindRange[]; text: FindRange[] } {
  const searchQuery = createInFileSearchQuery(query)
  return {
    state: collectInFileMatches(EditorState.create({ doc: source }), searchQuery),
    text: collectInFileMatches(cmText(source), searchQuery),
  }
}

function checkBoth(
  label: string,
  source: string,
  query: InFileFindQuery,
  expected: readonly FindRange[],
): void {
  const ranges = rangesFromBoth(source, query)
  check(`${label}: EditorState ranges`, sameRanges(ranges.state, expected))
  check(`${label}: CmText ranges`, sameRanges(ranges.text, expected))
  check(`${label}: both input forms agree`, sameRanges(ranges.state, ranges.text))
}

const literal = (text: string, caseSensitive = false): InFileFindQuery => ({
  text,
  mode: 'literal',
  caseSensitive,
})
const regexp = (text: string, caseSensitive = false): InFileFindQuery => ({
  text,
  mode: 'regexp',
  caseSensitive,
})

console.log('=== query contract and empty input ===')
const literalQuery = createInFileSearchQuery(literal('a\\nb'))
check('literal mode remains literal', literalQuery.literal && !literalQuery.regexp)
check('literal mode does not unescape backslash sequences', literalQuery.search === 'a\\nb')
check('replacement and whole-word matching stay disabled', literalQuery.replace === '' && !literalQuery.wholeWord)
const regexpQuery = createInFileSearchQuery(regexp('a.+b', true))
check('regexp mode enables regexp only', regexpQuery.regexp && !regexpQuery.literal)
check('case-sensitive flag is passed through', regexpQuery.caseSensitive)
checkBoth('empty query', 'anything', literal(''), [])
checkBoth('non-empty query with no result', 'anything', literal('missing'), [])

console.log('\n=== Unicode, case, and literal semantics ===')
checkBoth('ASCII literal', 'one two one', literal('one'), [
  { from: 0, to: 3 },
  { from: 8, to: 11 },
])
checkBoth('Chinese literal', '查找中文，再查找', literal('查找'), [
  { from: 0, to: 2 },
  { from: 6, to: 8 },
])
checkBoth('composed and decomposed characters', 'café cafe\u0301', literal('é'), [
  { from: 3, to: 4 },
  { from: 8, to: 10 },
])
checkBoth('case-insensitive default', 'Fluid fluid FLUID', literal('fluid'), [
  { from: 0, to: 5 },
  { from: 6, to: 11 },
  { from: 12, to: 17 },
])
checkBoth('case-sensitive toggle', 'Fluid fluid FLUID', literal('Fluid', true), [
  { from: 0, to: 5 },
])
checkBoth('regexp punctuation stays literal in text mode', 'x .* [a] y .* [a]', literal('.* [a]'), [
  { from: 2, to: 8 },
  { from: 11, to: 17 },
])
checkBoth('backslash-n stays literal in text mode', 'a\\nb\na\nb', literal('a\\nb'), [
  { from: 0, to: 4 },
])

console.log('\n=== regexp validity and zero-width termination ===')
checkBoth('valid regexp', 'foo fluid far', regexp('f(?:oo|luid)'), [
  { from: 0, to: 3 },
  { from: 4, to: 9 },
])
const invalid = createInFileSearchQuery(regexp('['))
check('invalid regexp is reported by SearchQuery', !invalid.valid)
check('invalid regexp collects no ranges without throwing', collectInFileMatches(cmText('abc'), invalid).length === 0)
const zeroWidth = rangesFromBoth('aaaa', regexp('(?=a)'))
check('zero-width regexp terminates at every valid position', sameRanges(zeroWidth.text, [
  { from: 0, to: 0 },
  { from: 1, to: 1 },
  { from: 2, to: 2 },
  { from: 3, to: 3 },
]))
check('zero-width regexp only returns empty ranges', zeroWidth.text.every((range) => range.from === range.to))
check('zero-width regexp agrees across input forms', sameRanges(zeroWidth.state, zeroWidth.text))

console.log('\n=== current counter and circular navigation ===')
const navigationRanges: FindRange[] = [
  { from: 0, to: 2 },
  { from: 4, to: 6 },
  { from: 8, to: 10 },
]
check('current range resolves to a 1-based number', currentInFileMatch(navigationRanges, { from: 4, to: 6 }) === 2)
check('missing current range resolves to zero', currentInFileMatch(navigationRanges, { from: 1, to: 2 }) === 0)
check('null current range resolves to zero', currentInFileMatch(navigationRanges, null) === 0)
check('next wraps last to first', moveInFileFindCurrent(3, 3, 'next') === 1)
check('previous wraps first to last', moveInFileFindCurrent(1, 3, 'previous') === 3)
check('next from no current starts at first', moveInFileFindCurrent(0, 3, 'next') === 1)
check('previous from no current starts at last', moveInFileFindCurrent(0, 3, 'previous') === 3)
check('no results always has current zero', moveInFileFindCurrent(1, 0, 'next') === 0)

console.log('\n=== CodeMirror controlled query state ===')
let controlledState = EditorState.create({
  doc: 'one two one',
  extensions: [search()],
})
check('search extension starts with a clear query', getSearchQuery(controlledState).search === '')
check('clear query starts with a zero snapshot', JSON.stringify(snapshotInFileFindState(controlledState)) === JSON.stringify({
  current: 0,
  total: 0,
  error: null,
}))

const controlledQuery = createInFileSearchQuery(literal('one'))
controlledState = controlledState.update({
  effects: setSearchQuery.of(controlledQuery),
}).state
check('controlled query is stored in CodeMirror search state', getSearchQuery(controlledState).eq(controlledQuery))

const controlledRanges = collectInFileMatches(controlledState, controlledQuery)
const secondSelection = controlledState.update({
  selection: { anchor: controlledRanges[1].from, head: controlledRanges[1].to },
  annotations: Transaction.userEvent.of('select.search'),
})
check('programmatic match selection is tagged select.search', secondSelection.isUserEvent('select.search'))
controlledState = secondSelection.state
check('selected match produces a 1-based CodeMirror snapshot', JSON.stringify(snapshotInFileFindState(controlledState)) === JSON.stringify({
  current: 2,
  total: 2,
  error: null,
}))

const wrappedIndex = moveInFileFindCurrent(2, controlledRanges.length, 'next')
const wrappedRange = controlledRanges[wrappedIndex - 1]
controlledState = controlledState.update({
  selection: { anchor: wrappedRange.from, head: wrappedRange.to },
  annotations: Transaction.userEvent.of('select.search'),
}).state
check('wrapped CodeMirror selection resolves to the first result', snapshotInFileFindState(controlledState).current === 1)

const manualSelection = controlledState.update({
  selection: { anchor: 4, head: 7 },
  annotations: Transaction.userEvent.of('select.pointer'),
})
check('manual selection is not mistaken for select.search', !manualSelection.isUserEvent('select.search'))

const selectionBeforeClear = controlledState.selection.main
controlledState = controlledState.update({
  effects: setSearchQuery.of(createInFileSearchQuery(literal(''))),
}).state
check('clearing removes the CodeMirror search query', getSearchQuery(controlledState).search === '')
check('clearing produces a zero snapshot', JSON.stringify(snapshotInFileFindState(controlledState)) === JSON.stringify({
  current: 0,
  total: 0,
  error: null,
}))
check('clearing leaves the navigated source position intact',
  controlledState.selection.main.from === selectionBeforeClear.from
  && controlledState.selection.main.to === selectionBeforeClear.to)

controlledState = controlledState.update({
  effects: setSearchQuery.of(createInFileSearchQuery(regexp('['))),
}).state
check('invalid regexp surfaces a controlled error snapshot', JSON.stringify(snapshotInFileFindState(controlledState)) === JSON.stringify({
  current: 0,
  total: 0,
  error: 'invalid-regexp',
}))

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll in-file-find checks passed.')
