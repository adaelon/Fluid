// S-QTURN-1 deterministic checks. Run with:
//   node scripts/query-presentation-check.ts
// Node 24 strips TypeScript annotations; no Vue, DOM, socket or provider is needed.

import {
  activeQueryTurnSelection,
  appendQueryTurnPresentationSnapshot,
  completedQueryTurnSelection,
  defaultQueryTurnSelection,
  normalizeQueryTurnSelection,
  queryTurnEvidenceById,
  queryTurnSelectionFromKey,
  queryTurnSelectionKey,
  resetQueryTurnPresentation,
  setQueryTurnAnswerHtml,
} from '../src/queryPresentation.ts'
import type { QueryMap } from '../src/ghostTypes.ts'
import type { QueryEvidenceState } from '../src/queryState.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function mapFor(filePath: string): QueryMap {
  return {
    actors: [{ id: 'file', name: filePath, role: '本轮文件', boundary: 'inside-file' }],
    direction: [],
    coreFunctionIds: [],
    supportingFunctionIds: [],
    walkthrough: {
      title: '直接作用',
      input: filePath,
      steps: [{ text: '核对本轮源码。', evidenceIds: ['E1'] }],
    },
    evidence: [{ id: 'E1', filePath, startLine: 2, endLine: 4, symbol: 'answer' }],
  }
}

console.log('=== active/completed focus selection state machine ===')
check('an empty trace has no focused turn', defaultQueryTurnSelection(0, false) === null)
let selection = activeQueryTurnSelection()
check('submitting the first question selects the active turn', selection.kind === 'active')
check(
  'streaming keeps the active turn selected',
  normalizeQueryTurnSelection(selection, 0, true)?.kind === 'active',
)
check(
  'an errored partial answer remains the active turn',
  normalizeQueryTurnSelection(selection, 0, true)?.kind === 'active',
)

selection = defaultQueryTurnSelection(1, false)
check(
  'done selects the latest completed turn',
  selection?.kind === 'completed' && selection.index === 0,
)
selection = completedQueryTurnSelection(0, 2)
check('history navigation can select an older completed turn', selection?.kind === 'completed')
selection = activeQueryTurnSelection()
check('asking while viewing history returns focus to active', selection.kind === 'active')
selection = defaultQueryTurnSelection(3, false)
check(
  'the next done frame advances to the newest completed turn',
  selection?.kind === 'completed' && selection.index === 2,
)
check(
  'an out-of-range completed selection falls back to the latest turn',
  normalizeQueryTurnSelection({ kind: 'completed', index: 8 }, 3, false)?.kind ===
    'completed' &&
    normalizeQueryTurnSelection({ kind: 'completed', index: 8 }, 3, false)?.index === 2,
)
check(
  'picker values round-trip a completed turn',
  queryTurnSelectionKey(queryTurnSelectionFromKey('completed:1', 3, false)) === 'completed:1',
)
check(
  'picker cannot select a missing active turn',
  queryTurnSelectionFromKey('active', 2, false)?.kind === 'completed',
)

console.log('\n=== per-turn presentation snapshots stay index-aligned ===')
const webEvidence: QueryEvidenceState = {
  status: 'web-cited',
  sources: [{ title: 'Source A', url: 'https://example.com/a' }],
}
let snapshots = appendQueryTurnPresentationSnapshot([], mapFor('src/a.ts'), webEvidence)
snapshots = appendQueryTurnPresentationSnapshot(snapshots, mapFor('src/b.ts'), null)
check('one snapshot is appended for each completed turn', snapshots.length === 2)
check(
  'evidence metadata is copied into the completed-turn snapshot',
  snapshots[0]?.evidence?.sources !== webEvidence.sources &&
    snapshots[0]?.evidence?.sources[0]?.url === 'https://example.com/a',
)
check(
  'the same E# resolves through the selected historical turn map',
  queryTurnEvidenceById(snapshots, 0, 'E1')?.filePath === 'src/a.ts' &&
    queryTurnEvidenceById(snapshots, 1, 'E1')?.filePath === 'src/b.ts',
)
const rendered = setQueryTurnAnswerHtml(snapshots, ['<p>A</p>', '<p>B</p>'])
check(
  'rendered Markdown HTML remains aligned by completed turn index',
  rendered[0]?.answerHtml === '<p>A</p>' && rendered[1]?.answerHtml === '<p>B</p>',
)
check('adding rendered HTML does not mutate the prior snapshots', snapshots[0]?.answerHtml === '')

const reset = resetQueryTurnPresentation()
check('scope/reset clears the focused selection', reset.selection === null)
check('scope/reset clears every presentation snapshot', reset.snapshots.length === 0)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query-presentation checks passed.')
