// S-QHISTORY-1 deterministic checks. Run with:
//   npx tsx scripts/query-history-check.ts
// No DOM, browser, real socket or provider is needed.

import { readFileSync } from 'node:fs'
import {
  QueryHistoryApiError,
  createQueryThread,
  deleteQueryThread,
  forkQueryThreadCurrent,
  getQueryThread,
  listQueryThreads,
  type QueryThread,
  type QueryThreadListResponse,
  type QueryThreadSummary,
} from '../src/api.ts'
import type { QueryMap } from '../src/ghostTypes.ts'
import { currentQueryScope, selectedQueryScope } from '../src/queryState.ts'
import { renderQueryMarkdown } from '../src/render/markdown.ts'
import {
  createQueryWorkspace,
  sortQueryThreadSummaries,
  type QueryHistoryClient,
  type QueryWorkspaceStream,
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
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

function mapFor(filePath: string, evidenceId = 'E1'): QueryMap {
  return {
    actors: [{ id: 'file', name: filePath, role: '本轮文件', boundary: 'inside-file' }],
    direction: [],
    coreFunctionIds: [],
    supportingFunctionIds: [],
    walkthrough: {
      title: '直接作用',
      input: filePath,
      steps: [{ text: '核对本轮源码。', evidenceIds: [evidenceId] }],
    },
    evidence: [{ id: evidenceId, filePath, startLine: 2, endLine: 4 }],
  }
}

function thread(
  id: string,
  updatedAt: string,
  options: { path?: string; title?: string; freshness?: 'fresh' | 'stale' } = {},
): QueryThread {
  const path = options.path ?? 'src/a.ts'
  const question = options.title ?? `为什么读取 ${path}？`
  return {
    schemaVersion: 1,
    id,
    title: question,
    createdAt: '2026-08-10T09:00:00.000Z',
    updatedAt,
    scope: { kind: 'current', paths: [path] },
    sourceRevision: `revision:${path}`,
    originalQuestion: question,
    turns: [
      {
        question,
        answer: `因为 ${path} 是真相源。[E1]`,
        map: mapFor(path),
        evidence: {
          status: 'project-source',
          sources: [],
        },
        codeEvidenceIds: ['E1'],
        completedAt: updatedAt,
      },
    ],
    freshness: options.freshness ?? 'fresh',
    ...(options.freshness === 'stale' ? { staleReason: 'source-changed' as const } : {}),
  }
}

function summary(value: QueryThread): QueryThreadSummary {
  return {
    id: value.id,
    title: value.title,
    updatedAt: value.updatedAt,
    scope: value.scope,
    turnCount: value.turns.length,
    freshness: value.freshness,
    ...(value.staleReason ? { staleReason: value.staleReason } : {}),
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

function fakeStream(): QueryWorkspaceStream & { cancelCount: number } {
  return {
    cancelCount: 0,
    cancel() {
      this.cancelCount++
    },
  }
}

const older = thread('thread-older', '2026-08-10T10:00:00.000Z')
const newer = thread('thread-newer', '2026-08-10T11:00:00.000Z', {
  path: 'src/b.ts',
  title: '新线程标题',
})

const restoredMarkdown = [
  '## 恢复标题',
  '### 恢复细节',
  '**历史强调**',
  '',
  '---',
  '',
  '| 输入 | 输出 |',
  '| --- | --- |',
  '| 旧记录 | 规范数组 |',
].join('\n')

function legacyWireThread(value: QueryThread, id = value.id): unknown {
  return {
    ...value,
    id,
    turns: value.turns.map((turn) => ({
      ...turn,
      answer: restoredMarkdown,
      evidence: turn.evidence
        ? {
            status: turn.evidence.status,
            ...(turn.evidence.warning ? { warning: turn.evidence.warning } : {}),
          }
        : null,
    })),
  }
}

console.log('=== history REST clients ===')
const originalFetch = globalThis.fetch
const apiCalls: Array<{ url: string; method: string }> = []
globalThis.fetch = async (input, init) => {
  const url = String(input)
  const method = init?.method ?? 'GET'
  apiCalls.push({ url, method })
  if (method === 'DELETE') return new Response(null, { status: 204 })
  if (url === '/api/query-threads' && method === 'GET') {
    return Response.json({ threads: [summary(newer)], warnings: [] })
  }
  if (url === '/api/query-threads' && method === 'POST') {
    return Response.json(legacyWireThread(older, 'thread-created'))
  }
  if (url.endsWith('/fork-current')) {
    return Response.json(legacyWireThread(older, 'thread-forked'))
  }
  return Response.json(legacyWireThread(older))
}
const apiList = await listQueryThreads()
const apiCreated = await createQueryThread({
  scope: { kind: 'current', paths: ['src/a.ts'] },
  originalQuestion: older.originalQuestion,
})
const apiDetail = await getQueryThread('thread/older')
const apiForked = await forkQueryThreadCurrent('thread/older')
await deleteQueryThread('thread/older')
check('list/create/get/fork/delete use the project history REST endpoints and encoded ids', apiList.threads[0]?.id === newer.id && apiCreated.id === 'thread-created' && apiDetail.id === older.id && apiForked.id === 'thread-forked' && apiCalls.map((call) => `${call.method} ${call.url}`).join(',') === 'GET /api/query-threads,POST /api/query-threads,GET /api/query-threads/thread%2Folder,POST /api/query-threads/thread%2Folder/fork-current,DELETE /api/query-threads/thread%2Folder')
check('all three detail clients normalize legacy omitted sources to arrays', [apiCreated, apiDetail, apiForked].every((value) => Array.isArray(value.turns[0]?.evidence?.sources) && value.turns[0]?.evidence?.sources.length === 0))

globalThis.fetch = async () => Response.json({
  ...older,
  turns: older.turns.map((turn) => ({
    ...turn,
    evidence: { status: 'unverified', sources: 'not-an-array' },
  })),
})
let contractError: unknown
try {
  await getQueryThread('bad-sources')
} catch (error) {
  contractError = error
}
check('a present non-array sources value raises an explicit API contract error', contractError instanceof Error && contractError.name === 'QueryHistoryContractError' && contractError.message.includes('sources'))

globalThis.fetch = async () => new Response('query thread not found', { status: 404 })
let apiError: unknown
try {
  await getQueryThread('missing')
} catch (error) {
  apiError = error
}
check('history REST failures preserve status for deleted-thread recovery', apiError instanceof QueryHistoryApiError && apiError.status === 404)
globalThis.fetch = originalFetch

console.log('\n=== legacy REST detail restoration and Markdown ===')
const legacyHistory = createQueryWorkspace(client({
  get: async () => apiDetail,
}))
let legacyRestoreError: unknown
let legacyHtml = ''
try {
  await legacyHistory.selectHistoryThread(apiDetail.id)
  const restored = legacyHistory.traceSnapshots.value[0]
  legacyHtml = renderQueryMarkdown(
    legacyHistory.trace.value?.turns[0]?.answer ?? '',
    restored?.map.evidence ?? [],
  )
} catch (error) {
  legacyRestoreError = error
}
check('legacy omitted sources passes restoreThreadProjection as an empty array', legacyRestoreError === undefined && legacyHistory.traceSnapshots.value[0]?.evidence?.sources.length === 0)
check('restored original Markdown keeps h2/h3/strong/hr/table structure', ['<h2>', '<h3>', '<strong>', '<hr>', '<table>'].every((tag) => legacyHtml.includes(tag)))

console.log('=== project history summaries and warnings ===')
const ordered = sortQueryThreadSummaries([summary(older), summary(newer)])
check('summaries are newest-first without mutating the input', ordered.map((item) => item.id).join(',') === 'thread-newer,thread-older')

const history = createQueryWorkspace(client({
  list: async () => ({
    threads: [summary(older), summary(newer)],
    warnings: [{ file: 'broken.json', message: 'unknown schema version' }],
  }),
  get: async (id) => id === older.id ? older : newer,
}))
check('the initial project history load is accepted', await history.loadProjectHistory())
check('loaded summaries are newest-first and preserve backend titles', history.historySummaries.value[0]?.title === '新线程标题' && history.historySummaries.value[1]?.id === older.id)
check('bad records remain visible warnings without hiding valid neighbors', history.historyWarnings.value[0]?.file === 'broken.json' && history.historySummaries.value.length === 2)

console.log('\n=== persisted turn restoration and completion summary ===')
const selected = await history.selectHistoryThread(older.id)
check('selecting a persisted thread restores its complete trace', selected.kind === 'selected' && history.trace.value?.turns[0]?.answer === older.turns[0]?.answer)
check('persisted maps and evidence rebuild index-aligned presentation snapshots', history.traceSnapshots.value[0]?.map.evidence[0]?.filePath === 'src/a.ts' && history.traceSnapshots.value[0]?.evidence?.status === 'project-source')
check('the restored thread becomes the selected durable identity', history.selectedThread.value?.id === older.id && history.threadId.value === older.id)

const selectedRecord: QueryThread = {
  ...older,
  id: 'thread-selected',
  title: '文件集历史',
  scope: { kind: 'selected', paths: ['src/a.ts', 'src/b.ts'] },
  sourceRevision: 'revision:selected',
}
const selectedHistory = createQueryWorkspace(client({
  get: async () => selectedRecord,
}))
await selectedHistory.selectHistoryThread(selectedRecord.id)
check('selected-file-set history restores with canonical path-order identity', selectedHistory.scope.value === 'selected' && selectedHistory.canContinueSelectedThread(selectedQueryScope(['src/b.ts', 'src/a.ts'])))

const identity = currentQueryScope('src/a.ts', 'orientation-current')
check('a fresh restored thread can continue only from its matching active scope', history.canContinueSelectedThread(identity) && !history.canContinueSelectedThread(currentQueryScope('src/other.ts', 'orientation-other')))
history.question.value = '后续问题'
const followUp = history.beginRequest(identity)
check('continuing a restored thread keeps all persisted turns', followUp !== null && followUp.trace.turns.length === 1 && followUp.threadId === older.id)
if (!followUp) process.exit(1)
history.acceptFrame(followUp, { kind: 'map', reqId: followUp.requestId, map: mapFor('src/a.ts', 'E2') })
history.acceptFrame(followUp, { kind: 'delta', reqId: followUp.requestId, text: '后续答案。[E2]' })
history.acceptFrame(followUp, {
  kind: 'done',
  reqId: followUp.requestId,
  threadId: older.id,
  updatedAt: '2026-08-10T12:00:00.000Z',
})
check('done updates turnCount/updatedAt and moves the summary to the front', history.historySummaries.value[0]?.id === older.id && history.historySummaries.value[0]?.turnCount === 2 && history.historySummaries.value[0]?.updatedAt === '2026-08-10T12:00:00.000Z')

console.log('\n=== deleted selection and project-generation isolation ===')
const missing = createQueryWorkspace(client({
  list: async () => ({ threads: [summary(older)], warnings: [] }),
  get: async () => {
    throw new QueryHistoryApiError(404, 'query thread not found')
  },
}))
await missing.loadProjectHistory()
const missingResult = await missing.selectHistoryThread(older.id)
check('selecting a thread deleted by another actor removes the stale summary', missingResult.kind === 'missing' && missing.historySummaries.value.length === 0 && missing.selectedThread.value === null)
check('a deleted selection produces a visible history error', missing.historyError.value.includes('已删除'))

const oldLoad = deferred<QueryThreadListResponse>()
const newLoad = deferred<QueryThreadListResponse>()
let listCall = 0
const projects = createQueryWorkspace(client({
  list: () => ++listCall === 1 ? oldLoad.promise : newLoad.promise,
  get: async () => older,
}))
const oldProjectLoad = projects.loadProjectHistory()
const newProjectLoad = projects.replaceProject()
newLoad.resolve({ threads: [summary(newer)], warnings: [] })
check('the replacement project load succeeds', await newProjectLoad)
oldLoad.resolve({ threads: [summary(older)], warnings: [] })
await oldProjectLoad
check('a late old-project list cannot overwrite the replacement project', projects.historySummaries.value.map((item) => item.id).join(',') === newer.id)

const lateGet = deferred<QueryThread>()
const selections = createQueryWorkspace(client({
  list: async () => ({ threads: [summary(older)], warnings: [] }),
  get: () => lateGet.promise,
}))
await selections.loadProjectHistory()
const oldSelection = selections.selectHistoryThread(older.id)
selections.resetForProjectChange()
lateGet.resolve(older)
check('a late old-project detail cannot restore a thread into the new project', (await oldSelection).kind === 'ignored' && selections.selectedThread.value === null)

console.log('\n=== explicit deletion cancels an active selected request ===')
let deletedId = ''
const deletions = createQueryWorkspace(client({
  list: async () => ({ threads: [summary(older)], warnings: [] }),
  get: async () => older,
  delete: async (id) => {
    deletedId = id
  },
}))
await deletions.loadProjectHistory()
await deletions.selectHistoryThread(older.id)
deletions.question.value = '删除前的在途问题'
const active = deletions.beginRequest(identity)
if (!active) process.exit(1)
const stream = fakeStream()
deletions.attachStream(active.generation, stream)
check('deleting the selected thread succeeds', await deletions.deleteHistoryThread(older.id))
check('deletion cancels the in-flight request before removing the durable record', deletedId === older.id && stream.cancelCount === 1)
check('deleting the selected thread returns to the project history list', deletions.selectedThread.value === null && deletions.threadId.value === null && deletions.historySummaries.value.length === 0)

console.log('\n=== view wiring stays controller-owned ===')
const panelSource = readFileSync(new URL('../src/QueryPanel.vue', import.meta.url), 'utf8')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
check('QueryPanel consumes controller history actions instead of calling history APIs', panelSource.includes('workspace.selectHistoryThread') && panelSource.includes('workspace.deleteHistoryThread') && !panelSource.includes('listQueryThreads(') && !panelSource.includes('getQueryThread('))
check('dock/focus expose one shared project history selector and bad-record warning', panelSource.includes('data-testid="query-history-picker"') && panelSource.includes('historyWarnings'))
const projectResetIndex = appSource.indexOf('queryWorkspace.resetForProjectChange()')
const projectOpenIndex = appSource.indexOf('await openFolder(path)')
const projectLoadIndex = appSource.indexOf('await queryWorkspace.loadProjectHistory()', projectOpenIndex)
check('a project-root switch cancels old work before changing roots, then reloads history', projectResetIndex >= 0 && projectResetIndex < projectOpenIndex && projectLoadIndex > projectOpenIndex)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query-history checks passed.')
