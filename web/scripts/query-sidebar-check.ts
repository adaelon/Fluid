// S-QLEFT-1 deterministic checks. Run with:
//   node scripts/query-sidebar-check.ts
// Node 24 strips the erasable TypeScript syntax used by queryLayout.ts.

import { readFileSync } from 'node:fs'
import {
  initialQueryWorkspaceLayout,
  queryWorkspaceLayoutValid,
  reduceQueryWorkspaceLayout,
  type QueryLayoutAction,
  type QueryWorkspaceLayoutState,
} from '../src/queryLayout.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function step(
  state: QueryWorkspaceLayoutState,
  type: QueryLayoutAction['type'],
): QueryWorkspaceLayoutState {
  return reduceQueryWorkspaceLayout(state, { type })
}

console.log('=== Explorer / Query / hidden activity-bar transitions ===')
const initial = initialQueryWorkspaceLayout()
check(
  'the shell starts with Explorer visible and the query hidden',
  !initial.visible
    && initial.sidebarView === 'explorer'
    && initial.placement === 'dock'
    && initial.focusReturn === 'dock',
)
const explorerHidden = step(initial, 'activity-explorer')
check('the active Explorer button collapses the sidebar', explorerHidden.sidebarView === 'hidden')
check(
  'the collapsed Explorer button restores the sidebar',
  step(explorerHidden, 'activity-explorer').sidebarView === 'explorer',
)
const sidebarQuery = step(initial, 'activity-query')
check(
  'the Query activity moves the single surface into the sidebar',
  sidebarQuery.visible
    && sidebarQuery.sidebarView === 'query'
    && sidebarQuery.placement === 'sidebar',
)
const queryHidden = step(sidebarQuery, 'activity-query')
check(
  'the active Query activity collapses without inventing another placement',
  !queryHidden.visible
    && queryHidden.sidebarView === 'hidden'
    && queryHidden.placement === 'sidebar',
)
const explorerFromQuery = step(sidebarQuery, 'activity-explorer')
check(
  'Explorer replaces a sidebar Query instead of sharing the column',
  !explorerFromQuery.visible && explorerFromQuery.sidebarView === 'explorer',
)

console.log('\n=== status, toolbar, focus-return and Escape transitions ===')
const dock = step(initial, 'status-toggle')
check(
  'the status entry opens the query in the dock',
  dock.visible && dock.placement === 'dock' && dock.sidebarView === 'explorer',
)
check('the status entry hides an open dock', !step(dock, 'status-toggle').visible)
const movedSidebar = step(dock, 'move-sidebar')
check(
  'the dock toolbar moves Query into the mutually-exclusive sidebar',
  movedSidebar.visible
    && movedSidebar.placement === 'sidebar'
    && movedSidebar.sidebarView === 'query',
)
const movedDock = step(movedSidebar, 'move-dock')
check(
  'moving a sidebar Query to the dock restores Explorer',
  movedDock.visible
    && movedDock.placement === 'dock'
    && movedDock.sidebarView === 'explorer',
)
const sidebarFocus = step(movedSidebar, 'focus')
check(
  'focus entered from the sidebar remembers the sidebar return target',
  sidebarFocus.placement === 'focus' && sidebarFocus.focusReturn === 'sidebar',
)
const sidebarRestored = step(sidebarFocus, 'escape')
check(
  'Escape restores focus to the original sidebar placement',
  sidebarRestored.placement === 'sidebar' && sidebarRestored.sidebarView === 'query',
)
const dockFocus = step(dock, 'focus')
check(
  'focus entered from the dock remembers the dock return target',
  dockFocus.placement === 'focus' && dockFocus.focusReturn === 'dock',
)
check(
  'the restore tool returns focus to the dock',
  step(dockFocus, 'restore-focus').placement === 'dock',
)
const closedSidebarFocus = step(sidebarFocus, 'close')
check(
  'closing focused sidebar Query hides the reserved sidebar column',
  !closedSidebarFocus.visible
    && closedSidebarFocus.placement === 'sidebar'
    && closedSidebarFocus.sidebarView === 'hidden',
)

console.log('\n=== every reachable reducer result satisfies layout invariants ===')
const actions: QueryLayoutAction['type'][] = [
  'activity-explorer',
  'activity-query',
  'status-toggle',
  'move-sidebar',
  'move-dock',
  'focus',
  'restore-focus',
  'escape',
  'close',
]
const queue: QueryWorkspaceLayoutState[] = [initial]
const seen = new Set<string>()
while (queue.length > 0) {
  const state = queue.shift()!
  const key = JSON.stringify(state)
  if (seen.has(key)) continue
  seen.add(key)
  check(`reachable state ${key} is valid`, queryWorkspaceLayoutValid(state))
  for (const type of actions) queue.push(step(state, type))
}
check('the transition graph exercises multiple legal states', seen.size >= 8)
check(
  'a non-focus placement cannot retain a split-brain focus return target',
  !queryWorkspaceLayoutValid({
    visible: true,
    placement: 'dock',
    focusReturn: 'sidebar',
    sidebarView: 'explorer',
  }),
)

console.log('\n=== the Vue shell keeps one controller and one movable QueryPanel ===')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const activitySource = readFileSync(
  new URL('../src/shell/ActivityBar.vue', import.meta.url),
  'utf8',
)
const panelSource = readFileSync(new URL('../src/QueryPanel.vue', import.meta.url), 'utf8')
const styleSource = readFileSync(new URL('../src/styles.css', import.meta.url), 'utf8')
check(
  'App creates exactly one project-scoped query workspace',
  (appSource.match(/createQueryWorkspace\(\)/g) ?? []).length === 1,
)
check(
  'App renders exactly one stable QueryPanel consumer',
  (appSource.match(/<QueryPanel\b/g) ?? []).length === 1
    && appSource.includes('v-show="queryPanelOpen && current"'),
)
check(
  'presentation-only hide never resets the controller trace, input or request',
  !appSource.includes('queryWorkspace.resetForClose()'),
)
check(
  'ActivityBar exposes mutually exclusive Explorer and Query controls',
  activitySource.includes("toggleExplorer: []")
    && activitySource.includes("toggleQuery: []")
    && activitySource.includes("sidebarView === 'explorer'")
    && activitySource.includes("sidebarView === 'query'"),
)
check(
  'QueryPanel exposes sidebar history/detail navigation and movement tools',
  panelSource.includes("type QuerySidebarPane = 'history' | 'thread'")
    && panelSource.includes('data-testid="query-sidebar-history"')
    && panelSource.includes("emit('moveDock')")
    && panelSource.includes("emit('moveSidebar')"),
)
check(
  'the narrow sidebar owns overflow instead of widening the application shell',
  styleSource.includes('.query-sidebar-history-list')
    && styleSource.includes('.query-panel.sidebar')
    && styleSource.includes('overflow-x: hidden'),
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll query-sidebar checks passed.')
