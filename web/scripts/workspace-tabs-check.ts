// S-WTABS-1 deterministic checks. Run with:
//   npx tsx web/scripts/workspace-tabs-check.ts
// The pure state core and App wiring are checked without a browser or provider.

import { readFileSync } from 'node:fs'
import type { FileNode, ProjectReadingSnapshot } from '../src/api.ts'
import {
  acceptWorkspaceSourceLoad,
  activateWorkspaceTab,
  activeReadyWorkspaceFile,
  beginWorkspaceSourceLoad,
  closeWorkspaceTab,
  createWorkspaceSourceLoadState,
  markWorkspaceTabLoading,
  markWorkspaceTabReady,
  resetWorkspaceSourceLoads,
  restoreWorkspaceTabs,
  type WorkspaceTabsState,
} from '../src/workspaceState.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function file(path: string, lang: FileNode['lang'] = 'rs'): FileNode {
  return { path, name: path.split('/').pop() ?? path, lang }
}

function snapshot(openFiles: string[], activeFile: string | null): ProjectReadingSnapshot {
  return {
    expandedDirectories: [],
    openFiles,
    activeFile,
    readingPositions: {},
  }
}

const files = [
  file('src/a.rs'),
  file('src/b.rs'),
  file('src/c.rs'),
  file('README.md', 'md'),
]

console.log('=== partial restore and nearest active selection ===')
const partial = restoreWorkspaceTabs(
  files,
  snapshot(['src/a.rs', 'src/deleted.rs', 'src/b.rs', 'src/c.rs'], 'src/deleted.rs'),
)
check(
  'missing paths are filtered while surviving order and language are preserved',
  partial.openFiles.map((tab) => `${tab.path}:${tab.lang}`).join(',')
    === 'src/a.rs:rs,src/b.rs:rs,src/c.rs:rs',
)
check('a missing active tab chooses the equidistant right neighbor first', partial.activePath === 'src/b.rs')
check('restored tabs carry identity but no source payload', partial.openFiles.every((tab) => tab.source === null && tab.loadState === 'unloaded'))
check('filtered paths remain observable for the later warning layer', partial.skippedOpenFiles.join(',') === 'src/deleted.rs')

const leftFallback = restoreWorkspaceTabs(
  files,
  snapshot(['src/a.rs', 'src/gone-1.rs', 'src/gone-2.rs'], 'src/gone-2.rs'),
)
check('when no right survivor exists the nearest left survivor is selected', leftFallback.activePath === 'src/a.rs')

const allMissing = restoreWorkspaceTabs(
  files,
  snapshot(['src/gone-1.rs', 'src/gone-2.rs'], 'src/gone-1.rs'),
)
check('an entirely invalid tab set restores the vacuum state', allMissing.openFiles.length === 0 && allMissing.activePath === null)

console.log('\n=== close semantics stay right-neighbor first ===')
let closing = restoreWorkspaceTabs(
  files,
  snapshot(['src/a.rs', 'src/b.rs', 'src/c.rs'], 'src/b.rs'),
)
closing = closeWorkspaceTab(closing, 'src/b.rs')
check('closing the active middle tab activates its right neighbor', closing.activePath === 'src/c.rs')
closing = closeWorkspaceTab(closing, 'src/c.rs')
check('closing the active last tab falls back left', closing.activePath === 'src/a.rs')
closing = closeWorkspaceTab(closing, 'src/a.rs')
check('closing the final tab returns to vacuum', closing.activePath === null && closing.openFiles.length === 0)

console.log('\n=== one active source request and no background activation ===')
let tabs: WorkspaceTabsState = restoreWorkspaceTabs(
  files,
  snapshot(['src/a.rs', 'src/b.rs', 'src/c.rs'], 'src/b.rs'),
)
let loads = createWorkspaceSourceLoadState()
const fileRequests: string[] = []
const orientationRequests: string[] = []
const generationRequests: string[] = []

function startLoad(path: string) {
  fileRequests.push(path)
  const begun = beginWorkspaceSourceLoad(loads, path)
  loads = begun.state
  tabs = markWorkspaceTabLoading(tabs, path)
  return begun.request
}

function finishLoad(request: ReturnType<typeof startLoad>, source: string): void {
  if (!acceptWorkspaceSourceLoad(loads, request)) return
  tabs = markWorkspaceTabReady(tabs, request.path, source)
  const visible = activeReadyWorkspaceFile(tabs)
  if (visible?.path === request.path) {
    orientationRequests.push(request.path)
    generationRequests.push(request.path)
  }
}

const restoredActiveRequest = startLoad(tabs.activePath ?? '')
check('restoring three tabs schedules exactly one /api/file request', fileRequests.join(',') === 'src/b.rs')
check('background tabs stay unloaded before their first activation', tabs.openFiles.filter((tab) => tab.path !== 'src/b.rs').every((tab) => tab.loadState === 'unloaded'))
check('no orient/generate activation occurs before source arrival', orientationRequests.length === 0 && generationRequests.length === 0)
finishLoad(restoredActiveRequest, 'fn b() {}')
check('only the restored active tab enters orient/generate after loading', orientationRequests.join(',') === 'src/b.rs' && generationRequests.join(',') === 'src/b.rs')

tabs = activateWorkspaceTab(tabs, 'src/c.rs')
const firstBackgroundActivation = startLoad('src/c.rs')
finishLoad(firstBackgroundActivation, 'fn c() {}')
check('a background tab fetches and activates only on first click', fileRequests.join(',') === 'src/b.rs,src/c.rs' && orientationRequests.join(',') === 'src/b.rs,src/c.rs')

console.log('\n=== late response and project-generation guards ===')
tabs = restoreWorkspaceTabs(files, snapshot(['src/a.rs', 'src/b.rs'], 'src/a.rs'))
loads = resetWorkspaceSourceLoads(loads)
const slowA = startLoad('src/a.rs')
tabs = activateWorkspaceTab(tabs, 'src/b.rs')
const fastB = startLoad('src/b.rs')
finishLoad(fastB, 'fn b() {}')
const activationsAfterB = orientationRequests.length
finishLoad(slowA, 'fn a() {}')
check('a late inactive response may fill its cache but never steals activePath', tabs.activePath === 'src/b.rs' && tabs.openFiles.find((tab) => tab.path === 'src/a.rs')?.loadState === 'ready')
check('a late inactive response emits no visible orient/generate activation', orientationRequests.length === activationsAfterB && generationRequests.length === activationsAfterB)

const oldProject = startLoad('src/a.rs')
loads = resetWorkspaceSourceLoads(loads)
const beforeOldProjectResponse = tabs
finishLoad(oldProject, 'stale project bytes')
check('resetting the project generation rejects old-project source responses', tabs === beforeOldProjectResponse)

console.log('\n=== App wiring ===')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const restoreIndex = appSource.indexOf('restoreWorkspaceTabs(')
const activeLoadIndex = appSource.indexOf('void loadOpenFile(restored.activePath)', restoreIndex)
const loopLoad = /for\s*\([^)]*restored\.openFiles[^)]*\)[\s\S]{0,200}loadOpenFile/.test(appSource)
check('App restores tab identities then requests only the chosen active source', restoreIndex >= 0 && activeLoadIndex > restoreIndex && !loopLoad)
check('Editor and Markdown activation are gated by a ready non-null source', appSource.includes('const readyCurrent = computed') && appSource.includes('v-if="readyCurrent && readyCurrent.lang === \'md\'"') && appSource.includes('v-else-if="readyCurrent"'))
check('the source loader never assigns activePath from a response callback', appSource.includes('acceptWorkspaceSourceLoad') && !/await fetchFile\([\s\S]{0,300}activePath\.value\s*=/.test(appSource))
const switchCallIndex = appSource.indexOf('await openFolder(path)')
const switchInstallIndex = appSource.indexOf('installWorkspaceTabs(', switchCallIndex)
check('a rejected project-open request cannot clear the existing tab UI', switchCallIndex >= 0 && switchInstallIndex > switchCallIndex)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll workspace-tab checks passed.')
