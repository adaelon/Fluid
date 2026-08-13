// S-WAUTO-1 deterministic checks. Run with:
//   npx tsx web/scripts/workspace-controller-check.ts
// No DOM, browser, real clock or backend is needed.

import { readFileSync } from 'node:fs'
import type {
  FileNode,
  ProjectReadingSnapshot,
  ReadingAnchor,
} from '../src/api.ts'
import { saveCurrentWorkspace, WorkspaceApiError } from '../src/api.ts'
import {
  WORKSPACE_SCROLL_DEBOUNCE_MS,
  WORKSPACE_STRUCTURE_DEBOUNCE_MS,
  createWorkspaceController,
  type WorkspaceTimerDriver,
} from '../src/workspaceController.ts'
import { restoreWorkspaceReadingPositions } from '../src/workspaceState.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function snapshot(
  openFiles: string[],
  activeFile: string | null = openFiles[0] ?? null,
  readingPositions: Record<string, ReadingAnchor> = {},
): ProjectReadingSnapshot {
  return {
    expandedDirectories: openFiles.length > 0 ? ['src'] : [],
    openFiles,
    activeFile,
    readingPositions,
  }
}

function file(path: string, lang: FileNode['lang']): FileNode {
  return { path, name: path.split('/').pop() ?? path, lang }
}

interface Deferred<T> {
  promise: Promise<T>
  resolve(value: T): void
  reject(error: unknown): void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((accept, decline) => {
    resolve = accept
    reject = decline
  })
  return { promise, resolve, reject }
}

class FakeClock implements WorkspaceTimerDriver {
  now = 0
  private sequence = 0
  private timers = new Map<number, { due: number; callback: () => void }>()

  setTimeout(callback: () => void, delayMs: number): unknown {
    const id = ++this.sequence
    this.timers.set(id, { due: this.now + delayMs, callback })
    return id
  }

  clearTimeout(handle: unknown): void {
    if (typeof handle === 'number') this.timers.delete(handle)
  }

  nextDelay(): number | null {
    const due = Math.min(...Array.from(this.timers.values(), (timer) => timer.due))
    return Number.isFinite(due) ? due - this.now : null
  }

  advance(milliseconds: number): void {
    const target = this.now + milliseconds
    while (true) {
      const next = Array.from(this.timers.entries())
        .filter(([, timer]) => timer.due <= target)
        .sort((left, right) => left[1].due - right[1].due || left[0] - right[0])[0]
      if (!next) break
      this.now = next[1].due
      this.timers.delete(next[0])
      next[1].callback()
    }
    this.now = target
  }
}

async function settle(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

console.log('=== workspace save API fixture ===')
const originalFetch = globalThis.fetch
let savedRequest: { input: RequestInfo | URL; init?: RequestInit } | null = null
globalThis.fetch = async (input, init) => {
  savedRequest = { input, init }
  return new Response('old root', { status: 409 })
}
let staleStatus: number | null = null
try {
  await saveCurrentWorkspace({
    projectRoot: 'A',
    snapshot: snapshot(['src/a.rs']),
  })
} catch (error) {
  staleStatus = error instanceof WorkspaceApiError ? error.status : -1
} finally {
  globalThis.fetch = originalFetch
}
const savedInit = savedRequest?.init
check(
  'the API writes one complete PUT body with unload-safe keepalive when small',
  savedRequest?.input === '/api/workspace/current'
    && savedInit?.method === 'PUT'
    && savedInit.keepalive === true
    && JSON.parse(String(savedInit.body)).snapshot.openFiles.join(',') === 'src/a.rs',
)
check('the API fixture preserves a stale-root 409 for controller isolation', staleStatus === 409)

console.log('=== restoration projection and zero-save gate ===')
const files = [
  file('src/a.rs', 'rs'),
  file('src/wrong.rs', 'rs'),
  file('docs/readme.md', 'md'),
]
const restoredAnchors = restoreWorkspaceReadingPositions(files, {
  'src/a.rs': { kind: 'code', topLine: 3, offsetPx: -4, totalLines: 20 },
  'docs/readme.md': {
    kind: 'markdown',
    blockDigest: 'abc123',
    occurrence: 0,
    offsetPx: 2,
    scrollRatio: 0.4,
  },
  'src/deleted.rs': { kind: 'code', topLine: 1, offsetPx: 0, totalLines: 1 },
  'src/wrong.rs': {
    kind: 'markdown',
    blockDigest: 'abc123',
    occurrence: 0,
    offsetPx: 0,
    scrollRatio: 0,
  },
})
check(
  'only current-tree anchors with a matching reader kind survive',
  Object.keys(restoredAnchors.readingPositions).join(',') === 'src/a.rs,docs/readme.md',
)
check(
  'missing or reader-mismatched anchors remain observable for one warning',
  restoredAnchors.skippedReadingPositions.join(',') === 'src/deleted.rs,src/wrong.rs',
)

const restoreClock = new FakeClock()
const restoreSaves: ProjectReadingSnapshot[] = []
const restoreNotices: string[] = []
const restoring = createWorkspaceController({
  save: async ({ snapshot: value }) => {
    restoreSaves.push(value)
    return { saved: true }
  },
  timers: restoreClock,
  notify: (notice) => restoreNotices.push(notice.message),
})
check('the controller starts in booting', restoring.phase === 'booting')
restoring.beginRestore('A', snapshot(['src/a.rs']))
check('installing a project enters restoring', restoring.phase === 'restoring')
restoring.updateSnapshot(snapshot(['src/a.rs'], 'src/a.rs', restoredAnchors.readingPositions), 'scroll')
restoreClock.advance(5_000)
await settle()
check('restoration-time changes update memory without saving', restoreSaves.length === 0 && !restoring.dirty)
restoring.reportRestoreIssues({
  backendWarnings: 1,
  skippedDirectories: 2,
  skippedOpenFiles: 3,
  skippedReadingPositions: 4,
})
restoring.reportRestoreIssues({
  backendWarnings: 1,
  skippedDirectories: 2,
  skippedOpenFiles: 3,
  skippedReadingPositions: 4,
})
check(
  'one restore notice summarizes every degradation category and count',
  restoreNotices.length === 1
    && ['记录 1', '目录 2', '标签 3', '阅读位置 4'].every((part) => restoreNotices[0]?.includes(part)),
)
restoring.completeRestore()
check('a completed restoration enables ready state without an eager cleanup save', restoring.phase === 'ready' && restoreSaves.length === 0)

console.log('\n=== 150/400 ms debounce and latest-snapshot single flight ===')
const clock = new FakeClock()
const pendingSaves: Deferred<{ saved: true }>[] = []
const requests: Array<{ projectRoot: string; snapshot: ProjectReadingSnapshot }> = []
const controller = createWorkspaceController({
  save: (request) => {
    requests.push(request)
    const pending = deferred<{ saved: true }>()
    pendingSaves.push(pending)
    return pending.promise
  },
  timers: clock,
})
controller.beginRestore('A', snapshot([]))
controller.completeRestore()
controller.updateSnapshot(snapshot(['src/a.rs']), 'structure')
check('tree/tab changes use the frozen 150 ms debounce', WORKSPACE_STRUCTURE_DEBOUNCE_MS === 150 && clock.nextDelay() === 150)
clock.advance(149)
await settle()
check('a structure save does not start before 150 ms', requests.length === 0)
clock.advance(1)
await settle()
check('the first complete snapshot starts at 150 ms', requests.length === 1 && requests[0]?.snapshot.openFiles.join(',') === 'src/a.rs')

controller.updateSnapshot(snapshot(['src/a.rs', 'src/b.rs'], 'src/b.rs'), 'structure')
clock.advance(150)
await settle()
controller.updateSnapshot(snapshot(['src/a.rs', 'src/b.rs', 'src/c.rs'], 'src/c.rs'), 'scroll')
check('reader scroll uses the frozen 400 ms debounce', WORKSPACE_SCROLL_DEBOUNCE_MS === 400 && clock.nextDelay() === 400)
clock.advance(400)
await settle()
check('changes cannot open a second save while one is in flight', requests.length === 1)
pendingSaves[0]?.resolve({ saved: true })
await settle()
check(
  'completion drains exactly the latest merged whole snapshot',
  requests.length === 2
    && requests[1]?.snapshot.openFiles.join(',') === 'src/a.rs,src/b.rs,src/c.rs'
    && requests[1]?.snapshot.activeFile === 'src/c.rs',
)
pendingSaves[1]?.resolve({ saved: true })
await settle()
check('the latest successful response clears dirty state', !controller.dirty)

const mixedClock = new FakeClock()
const mixedRequests: ProjectReadingSnapshot[] = []
const mixed = createWorkspaceController({
  save: async ({ snapshot: value }) => {
    mixedRequests.push(value)
    return { saved: true }
  },
  timers: mixedClock,
})
mixed.beginRestore('A', snapshot([]))
mixed.completeRestore()
mixed.updateSnapshot(snapshot(['src/structure.rs']), 'structure')
mixedClock.advance(100)
mixed.updateSnapshot(snapshot(['src/structure.rs'], 'src/structure.rs', {
  'src/structure.rs': { kind: 'code', topLine: 2, offsetPx: 0, totalLines: 10 },
}), 'scroll')
check('a later scroll cannot postpone an already scheduled structure save', mixedClock.nextDelay() === 50)
mixedClock.advance(50)
await settle()
check(
  'the first due channel still submits the newest complete snapshot',
  mixedRequests.length === 1
    && mixedRequests[0]?.readingPositions['src/structure.rs']?.kind === 'code',
)

console.log('\n=== old-root response isolation and switch flush ===')
const switchClock = new FakeClock()
const switchRequests: Array<{ projectRoot: string; snapshot: ProjectReadingSnapshot }> = []
const switchPending: Deferred<{ saved: true }>[] = []
const switchNotices: string[] = []
const switching = createWorkspaceController({
  save: (request) => {
    switchRequests.push(request)
    const pending = deferred<{ saved: true }>()
    switchPending.push(pending)
    return pending.promise
  },
  timers: switchClock,
  notify: (notice) => switchNotices.push(notice.message),
})
switching.beginRestore('old-root', snapshot([]))
switching.completeRestore()
switching.updateSnapshot(snapshot(['src/old.rs']), 'structure')
switchClock.advance(150)
await settle()
switching.beginRestore('new-root', snapshot([]))
switching.completeRestore()
switching.updateSnapshot(snapshot(['src/new.rs']), 'structure')
switchClock.advance(150)
await settle()
check('the old-root request remains the only in-flight write', switchRequests.length === 1 && switchRequests[0]?.projectRoot === 'old-root')
switchPending[0]?.reject(new WorkspaceApiError(409, 'old root'))
await settle()
check(
  'an old-root 409 starts but cannot clear the new root latest save',
  switchRequests.length === 2
    && switchRequests[1]?.projectRoot === 'new-root'
    && switching.dirty
    && switchNotices.length === 0,
)
switchPending[1]?.resolve({ saved: true })
await settle()
check('only the matching new-root completion clears dirty', !switching.dirty)

switching.updateSnapshot(snapshot(['src/new.rs', 'src/two.rs'], 'src/two.rs'), 'structure')
switching.beginSwitch()
const flushed = switching.flush()
await settle()
check('beginSwitch exposes switching and flush bypasses the debounce', switching.phase === 'switching' && switchRequests.length === 3)
switchPending[2]?.resolve({ saved: true })
await flushed
switching.cancelSwitch()
check('a rejected root switch can return the preserved workspace to ready', switching.phase === 'ready' && !switching.dirty)

console.log('\n=== save failure stays dirty and notifies once ===')
const failureClock = new FakeClock()
const failureNotices: string[] = []
let failureAttempts = 0
const failing = createWorkspaceController({
  save: async () => {
    failureAttempts++
    throw new Error('disk full')
  },
  timers: failureClock,
  notify: (notice) => failureNotices.push(notice.message),
})
failing.beginRestore('A', snapshot([]))
failing.completeRestore()
failing.updateSnapshot(snapshot(['src/a.rs']), 'structure')
failureClock.advance(150)
await settle()
check('a failed save keeps dirty and emits one non-blocking error', failing.dirty && failureAttempts === 1 && failureNotices.length === 1)
failing.updateSnapshot(snapshot(['src/a.rs', 'src/b.rs'], 'src/b.rs'), 'structure')
failureClock.advance(150)
await settle()
check('a later user change retries without repeating the same notice', failureAttempts === 2 && failureNotices.length === 1)

console.log('\n=== App ownership and non-blocking presentation wiring ===')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const editorSource = readFileSync(new URL('../src/Editor.vue', import.meta.url), 'utf8')
const markdownSource = readFileSync(new URL('../src/MarkdownView.vue', import.meta.url), 'utf8')
const styleSource = readFileSync(new URL('../src/styles.css', import.meta.url), 'utf8')
const flushIndex = appSource.indexOf('await workspaceController.flush()')
const openIndex = appSource.indexOf('await openFolder(path)', flushIndex)
check(
  'App creates one controller backed by the workspace save API',
  appSource.includes('const workspaceController = createWorkspaceController(')
    && appSource.includes('save: saveCurrentWorkspace'),
)
check('project switching flushes the old complete snapshot before open', flushIndex >= 0 && openIndex > flushIndex)
check(
  'tab actions capture the visible reader before changing activePath',
  /function activate\([^)]*\)[\s\S]{0,220}captureActiveReadingAnchor\(\)[\s\S]{0,220}activateWorkspaceTab/.test(appSource)
    && /function closeTab\([^)]*\)[\s\S]{0,220}captureActiveReadingAnchor\(\)[\s\S]{0,220}closeWorkspaceTab/.test(appSource),
)
check(
  'both readers emit path-owned anchors into one App handler',
  appSource.includes('@reading-anchor="recordReadingAnchor"')
    && appSource.includes('@reading-interaction="beginReadingInteraction"')
    && editorSource.includes("'reading-anchor': [path: string, anchor: CodeReadingAnchor]")
    && markdownSource.includes("'reading-anchor': [path: string, anchor: MarkdownReadingAnchor]"),
)
check(
  'mounted readers receive persisted anchors through the exposed restore handle',
  appSource.includes('restoreActiveReadingAnchor')
    && appSource.includes('surface?.restoreReadingAnchor?.(anchor)')
    && appSource.includes('@reading-restore-settled="finishReadingRestore"'),
)
check(
  'pagehide and component teardown both trigger best-effort workspace flush',
  appSource.includes("addEventListener('pagehide', flushWorkspaceBestEffort)")
    && appSource.includes("removeEventListener('pagehide', flushWorkspaceBestEffort)")
    && appSource.includes('void workspaceController.flush()'),
)
check(
  'the aggregate notice is an auto-dismissed non-modal status surface',
  appSource.includes('role="status"')
    && appSource.includes('workspaceNotice')
    && styleSource.includes('.workspace-notice')
    && !appSource.includes('alert('),
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll workspace-controller checks passed.')
