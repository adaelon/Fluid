// S-QSTALE-1 deterministic checks. Run with:
//   npx tsx scripts/query-stale-check.ts
// No DOM, browser, real socket or provider is needed.

import { readFileSync } from 'node:fs'
import {
  forkQueryThreadCurrent,
  type QueryThread,
  type QueryThreadSummary,
} from '../src/api.ts'
import type { QueryMap } from '../src/ghostTypes.ts'
import {
  queryCodeEvidenceNavigationEnabled,
  queryStaleReasonMessage,
} from '../src/queryEvidence.ts'
import { currentQueryScope } from '../src/queryState.ts'
import {
  createQueryWorkspace,
  type QueryHistoryClient,
} from '../src/queryWorkspace.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve
  })
  return { promise, resolve }
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
    evidence: [{ id: 'E1', filePath, startLine: 2, endLine: 4 }],
  }
}

function staleThread(
  reason: 'source-changed' | 'source-missing',
): QueryThread {
  return {
    schemaVersion: 1,
    id: `stale-${reason}`,
    title: '原始问题',
    createdAt: '2026-08-11T01:00:00.000Z',
    updatedAt: '2026-08-11T01:01:00.000Z',
    scope: { kind: 'current', paths: ['src/a.ts'] },
    sourceRevision: 'old-revision',
    originalQuestion: '原始问题',
    turns: [
      {
        question: '原始问题',
        answer: '旧回答 [E1]',
        map: mapFor('src/a.ts'),
        evidence: {
          status: 'web-cited',
          sources: [{ title: '网页来源', url: 'https://example.com/source' }],
        },
        codeEvidenceIds: ['E1'],
        completedAt: '2026-08-11T01:01:00.000Z',
      },
    ],
    freshness: 'stale',
    staleReason: reason,
  }
}

function summary(thread: QueryThread): QueryThreadSummary {
  return {
    id: thread.id,
    title: thread.title,
    updatedAt: thread.updatedAt,
    scope: thread.scope,
    turnCount: thread.turns.length,
    freshness: thread.freshness,
    ...(thread.staleReason ? { staleReason: thread.staleReason } : {}),
  }
}

function client(overrides: Partial<QueryHistoryClient> = {}): QueryHistoryClient {
  return {
    list: async () => ({ threads: [], warnings: [] }),
    get: async () => {
      throw new Error('unexpected get')
    },
    create: async () => {
      throw new Error('unexpected create')
    },
    delete: async () => {
      throw new Error('unexpected delete')
    },
    forkCurrent: async () => {
      throw new Error('unexpected fork')
    },
    ...overrides,
  }
}

console.log('=== fork-current REST client ===')
const changed = staleThread('source-changed')
const forked: QueryThread = {
  ...changed,
  id: 'fresh-fork',
  createdAt: '2026-08-11T02:00:00.000Z',
  updatedAt: '2026-08-11T02:00:00.000Z',
  sourceRevision: 'new-revision',
  turns: [],
  freshness: 'fresh',
  staleReason: undefined,
}
const originalFetch = globalThis.fetch
let forkRequest = ''
globalThis.fetch = async (input, init) => {
  forkRequest = `${init?.method ?? 'GET'} ${String(input)}`
  return Response.json(forked, { status: 201 })
}
const apiFork = await forkQueryThreadCurrent('stale/source-changed')
check(
  'fork client posts to the encoded current-source endpoint',
  forkRequest === 'POST /api/query-threads/stale%2Fsource-changed/fork-current',
)
check('fork client returns the fresh zero-turn record', apiFork.id === forked.id && apiFork.turns.length === 0)
globalThis.fetch = originalFetch

console.log('\n=== stale read-only and source-changed fork ===')
let forkCalls = 0
const workspace = createQueryWorkspace(client({
  list: async () => ({ threads: [summary(changed)], warnings: [] }),
  get: async () => changed,
  forkCurrent: async (threadId) => {
    forkCalls++
    if (threadId !== changed.id) throw new Error('wrong fork source')
    return forked
  },
}))
await workspace.loadProjectHistory()
await workspace.selectHistoryThread(changed.id)
check(
  'a stale thread cannot continue even when its path is active',
  !workspace.canContinueSelectedThread(currentQueryScope('src/a.ts', 'new-revision')),
)
check(
  'source-changed explains that old code evidence cannot return to current source',
  queryStaleReasonMessage(changed.staleReason).includes('源码已变更')
    && queryStaleReasonMessage(changed.staleReason).includes('不可回切'),
)
check(
  'stale disables code evidence while fresh or unselected history permits it',
  !queryCodeEvidenceNavigationEnabled(changed.freshness)
    && queryCodeEvidenceNavigationEnabled(forked.freshness)
    && queryCodeEvidenceNavigationEnabled(undefined),
)
check('source-changed can fork against current bytes', await workspace.forkSelectedThreadCurrent())
check(
  'fork replaces the selection with a new id, zero turns and new revision',
  workspace.selectedThread.value?.id === forked.id
    && workspace.selectedThread.value.turns.length === 0
    && workspace.selectedThread.value.sourceRevision === 'new-revision'
    && workspace.selectedThread.value.sourceRevision !== changed.sourceRevision,
)
check(
  'fork preserves the old record and pre-fills only the copied original question',
  forkCalls === 1
    && workspace.historySummaries.value.some((item) => item.id === changed.id && item.freshness === 'stale')
    && workspace.historySummaries.value.some((item) => item.id === forked.id && item.turnCount === 0)
    && workspace.question.value === changed.originalQuestion,
)
check(
  'the fresh fork can continue only from its matching explicit scope',
  workspace.canContinueSelectedThread(currentQueryScope('src/a.ts', 'new-revision'))
    && !workspace.canContinueSelectedThread(currentQueryScope('src/other.ts', 'new-revision')),
)

const lateFork = deferred<QueryThread>()
const replacedProject = createQueryWorkspace(client({
  get: async () => changed,
  forkCurrent: () => lateFork.promise,
}))
await replacedProject.selectHistoryThread(changed.id)
const oldProjectFork = replacedProject.forkSelectedThreadCurrent()
replacedProject.resetForProjectChange()
lateFork.resolve(forked)
check(
  'a late fork result cannot enter a replacement project',
  !(await oldProjectFork)
    && replacedProject.selectedThread.value === null
    && replacedProject.historySummaries.value.length === 0,
)

console.log('\n=== source-missing remains read-only without fork ===')
const missing = staleThread('source-missing')
let missingForkCalls = 0
const missingWorkspace = createQueryWorkspace(client({
  get: async () => missing,
  forkCurrent: async () => {
    missingForkCalls++
    return forked
  },
}))
await missingWorkspace.selectHistoryThread(missing.id)
check(
  'source-missing names the unavailable range',
  queryStaleReasonMessage(missing.staleReason).includes('范围文件缺失'),
)
check(
  'source-missing never offers or calls fork-current',
  !(await missingWorkspace.forkSelectedThreadCurrent()) && missingForkCalls === 0,
)

console.log('\n=== view wiring keeps only code evidence inert ===')
const panelSource = readFileSync(new URL('../src/QueryPanel.vue', import.meta.url), 'utf8')
const mapSource = readFileSync(new URL('../src/QueryMapView.vue', import.meta.url), 'utf8')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
check(
  'stale disables the composer and guards answer E# before opening evidence',
  panelSource.includes(':disabled="streaming || historyReadOnly"')
    && panelSource.includes('if (!codeEvidenceEnabled.value)')
    && panelSource.includes(':code-evidence-enabled="codeEvidenceEnabled"'),
)
check(
  'direction-map E# controls are disabled and labeled as old-source evidence',
  mapSource.includes(':disabled="!codeEvidenceEnabled"')
    && mapSource.includes('旧源码证据，当前不可回切'),
)
check(
  'web sources remain ordinary external links in stale history',
  panelSource.includes('<a :href="source.url" target="_blank" rel="noopener noreferrer">'),
)
check(
  'fresh range restoration remains an explicit user action',
  panelSource.includes("@click=\"emit('restoreScope', selectedThread.scope)\"")
    && appSource.includes('@restore-scope="restoreQueryScope"'),
)
check(
  'only source-changed renders the current-source fork action',
  panelSource.includes("selectedThread.staleReason === 'source-changed'")
    && panelSource.includes('workspace.forkSelectedThreadCurrent()')
    && panelSource.includes('基于当前源码新建追问'),
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query stale checks passed.')
