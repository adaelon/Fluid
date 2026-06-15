// S10b-cap validation (B2, deterministic). Exercises the pure snapshot core —
// buildQueryContext — with assertions, no store / Vue / sockets / browser.
// Run with: node scripts/query-context-check.ts  (Node 24 strips TS types).
//
// The Editor→App→QueryPanel emit bridge and real WS payload are browser-verified
// (A2); this script locks the roster/capsule snapshot logic those rely on.

import { buildQueryContext, EMPTY_QUERY_CONTEXT } from '../src/queryContext.ts'

let failures = 0
function check(label: string, cond: boolean): void {
  if (cond) {
    console.log(`  PASS  ${label}`)
  } else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

const roster = [
  { id: 'load#1', name: 'load' },
  { id: 'parse#10', name: 'parse' },
  { id: 'save#30', name: 'save' },
]

console.log('=== empty roster → empty context ===')
const empty = buildQueryContext([], () => 'x')
check('no roster names', empty.roster.length === 0)
check('no capsules', empty.capsules.length === 0)
check('EMPTY_QUERY_CONTEXT is empty', EMPTY_QUERY_CONTEXT.roster.length === 0 && EMPTY_QUERY_CONTEXT.capsules.length === 0)

console.log('\n=== full generation → every capsule, roster order ===')
const summaries = new Map<string, string>([
  ['load#1', 'reads the file'],
  ['parse#10', 'parses the source'],
  ['save#30', 'writes the cache'],
])
const full = buildQueryContext(roster, (id) => summaries.get(id))
check('roster lists all names in order', full.roster.join(',') === 'load,parse,save')
check('all three capsules present', full.capsules.length === 3)
check('capsule carries name + summary', full.capsules[1].name === 'parse' && full.capsules[1].summary === 'parses the source')
check('capsule order follows roster', full.capsules.map((c) => c.name).join(',') === 'load,parse,save')

console.log('\n=== partial generation → only settled capsules, roster still full ===')
const partial = buildQueryContext(roster, (id) => (id === 'parse#10' ? 'parses the source' : undefined))
check('roster still lists all three', partial.roster.length === 3)
check('only the generated capsule included', partial.capsules.length === 1 && partial.capsules[0].name === 'parse')

console.log('\n=== empty summary string is skipped (no blank capsules) ===')
const blank = buildQueryContext(roster, (id) => (id === 'load#1' ? '' : undefined))
check('empty-string summary not emitted', blank.capsules.length === 0)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query-context checks passed.')
