import type {
  FileNode,
  Lang,
  ProjectReadingSnapshot,
  ReadingAnchor,
} from './api.ts'
import {
  normalizeCodeReadingAnchor,
  normalizeMarkdownReadingAnchor,
} from './readingAnchor.ts'

export type WorkspaceFileLoadState = 'unloaded' | 'loading' | 'ready' | 'error'

export interface WorkspaceOpenFile {
  path: string
  lang: Lang
  source: string | null
  loadState: WorkspaceFileLoadState
}

export interface WorkspaceReadyOpenFile extends WorkspaceOpenFile {
  source: string
  loadState: 'ready'
}

export interface WorkspaceTabsState {
  openFiles: WorkspaceOpenFile[]
  activePath: string | null
}

export interface RestoredWorkspaceTabs extends WorkspaceTabsState {
  skippedOpenFiles: string[]
}

export interface RestoredWorkspaceReadingPositions {
  readingPositions: Record<string, ReadingAnchor>
  skippedReadingPositions: string[]
}

function unloadedTab(node: FileNode): WorkspaceOpenFile {
  return {
    path: node.path,
    lang: node.lang,
    source: null,
    loadState: 'unloaded',
  }
}

function chooseRestoredActivePath(
  persistedPaths: readonly string[],
  persistedActivePath: string | null,
  survivingPaths: ReadonlySet<string>,
): string | null {
  if (persistedActivePath && survivingPaths.has(persistedActivePath)) {
    return persistedActivePath
  }

  const activeIndex = persistedActivePath === null
    ? -1
    : persistedPaths.indexOf(persistedActivePath)
  if (activeIndex >= 0) {
    for (let distance = 1; distance < persistedPaths.length; distance++) {
      const right = persistedPaths[activeIndex + distance]
      if (right && survivingPaths.has(right)) return right

      const left = persistedPaths[activeIndex - distance]
      if (left && survivingPaths.has(left)) return left
    }
  }

  return survivingPaths.values().next().value ?? null
}

/** Project a persisted ordered tab list onto the current flat file tree. Missing
 * paths are omitted, while a missing active path falls to the nearest surviving
 * original neighbor (right before left at the same distance). */
export function restoreWorkspaceTabs(
  files: readonly FileNode[],
  snapshot: ProjectReadingSnapshot | null,
): RestoredWorkspaceTabs {
  if (!snapshot) {
    return { openFiles: [], activePath: null, skippedOpenFiles: [] }
  }

  const nodesByPath = new Map(files.map((node) => [node.path, node]))
  const seen = new Set<string>()
  const openFiles: WorkspaceOpenFile[] = []
  const skippedOpenFiles: string[] = []

  for (const path of snapshot.openFiles) {
    const node = nodesByPath.get(path)
    if (!node || seen.has(path)) {
      skippedOpenFiles.push(path)
      continue
    }
    seen.add(path)
    openFiles.push(unloadedTab(node))
  }

  const survivingPaths = new Set(openFiles.map((tab) => tab.path))
  return {
    openFiles,
    activePath: chooseRestoredActivePath(
      snapshot.openFiles,
      snapshot.activeFile,
      survivingPaths,
    ),
    skippedOpenFiles,
  }
}

/** Project persisted reader anchors onto the current file tree. The backend
 * validates the wire shape, while this final frontend gate also rejects paths
 * that disappeared and anchors meant for the other reader kind. */
export function restoreWorkspaceReadingPositions(
  files: readonly FileNode[],
  persistedPositions: Readonly<Record<string, ReadingAnchor>>,
): RestoredWorkspaceReadingPositions {
  const nodesByPath = new Map(files.map((node) => [node.path, node]))
  const readingPositions: Record<string, ReadingAnchor> = {}
  const skippedReadingPositions: string[] = []

  for (const [path, anchor] of Object.entries(persistedPositions)) {
    const node = nodesByPath.get(path)
    const normalized = node?.lang === 'md'
      ? normalizeMarkdownReadingAnchor(anchor)
      : node
        ? normalizeCodeReadingAnchor(anchor)
        : null
    if (!node || !normalized) {
      skippedReadingPositions.push(path)
      continue
    }
    readingPositions[path] = normalized
  }

  return { readingPositions, skippedReadingPositions }
}

/** Add a new unloaded tab or synchronously activate its existing identity. */
export function openWorkspaceTab<T extends WorkspaceTabsState>(
  state: T,
  node: FileNode,
): T {
  if (state.openFiles.some((tab) => tab.path === node.path)) {
    return activateWorkspaceTab(state, node.path)
  }
  return {
    ...state,
    openFiles: [...state.openFiles, unloadedTab(node)],
    activePath: node.path,
  }
}

export function activateWorkspaceTab<T extends WorkspaceTabsState>(
  state: T,
  path: string,
): T {
  if (!state.openFiles.some((tab) => tab.path === path)) return state
  return { ...state, activePath: path }
}

/** Preserve the existing IDE rule: closing the active tab selects the tab now
 * occupying its index (the old right neighbor), then the left, then vacuum. */
export function closeWorkspaceTab<T extends WorkspaceTabsState>(
  state: T,
  path: string,
): T {
  const index = state.openFiles.findIndex((tab) => tab.path === path)
  if (index < 0) return state

  const openFiles = state.openFiles.filter((_, candidate) => candidate !== index)
  if (state.activePath !== path) return { ...state, openFiles }
  const next = openFiles[index] ?? openFiles[index - 1] ?? null
  return { ...state, openFiles, activePath: next?.path ?? null }
}

function updateWorkspaceTab<T extends WorkspaceTabsState>(
  state: T,
  path: string,
  update: (tab: WorkspaceOpenFile) => WorkspaceOpenFile,
): T {
  let changed = false
  const openFiles = state.openFiles.map((tab) => {
    if (tab.path !== path) return tab
    changed = true
    return update(tab)
  })
  return changed ? { ...state, openFiles } : state
}

export function markWorkspaceTabLoading<T extends WorkspaceTabsState>(
  state: T,
  path: string,
): T {
  return updateWorkspaceTab(state, path, (tab) => ({
    ...tab,
    source: null,
    loadState: 'loading',
  }))
}

export function markWorkspaceTabReady<T extends WorkspaceTabsState>(
  state: T,
  path: string,
  source: string,
): T {
  return updateWorkspaceTab(state, path, (tab) => ({
    ...tab,
    source,
    loadState: 'ready',
  }))
}

export function markWorkspaceTabError<T extends WorkspaceTabsState>(
  state: T,
  path: string,
): T {
  return updateWorkspaceTab(state, path, (tab) => ({
    ...tab,
    source: null,
    loadState: 'error',
  }))
}

export function activeReadyWorkspaceFile(
  state: WorkspaceTabsState,
): WorkspaceReadyOpenFile | null {
  const active = state.openFiles.find((tab) => tab.path === state.activePath)
  return active?.loadState === 'ready' && active.source !== null
    ? active as WorkspaceReadyOpenFile
    : null
}

export interface WorkspaceSourceLoadState {
  generation: number
  sequence: number
  latestRequestByPath: ReadonlyMap<string, number>
}

export interface WorkspaceSourceLoadRequest {
  generation: number
  requestId: number
  path: string
}

export function createWorkspaceSourceLoadState(): WorkspaceSourceLoadState {
  return {
    generation: 0,
    sequence: 0,
    latestRequestByPath: new Map(),
  }
}

/** Invalidate every request belonging to the previous project root. */
export function resetWorkspaceSourceLoads(
  state: WorkspaceSourceLoadState,
): WorkspaceSourceLoadState {
  return {
    generation: state.generation + 1,
    sequence: state.sequence,
    latestRequestByPath: new Map(),
  }
}

export function beginWorkspaceSourceLoad(
  state: WorkspaceSourceLoadState,
  path: string,
): { state: WorkspaceSourceLoadState; request: WorkspaceSourceLoadRequest } {
  const requestId = state.sequence + 1
  const latestRequestByPath = new Map(state.latestRequestByPath)
  latestRequestByPath.set(path, requestId)
  return {
    state: {
      generation: state.generation,
      sequence: requestId,
      latestRequestByPath,
    },
    request: {
      generation: state.generation,
      requestId,
      path,
    },
  }
}

/** A response is writable only for its original project generation and while it
 * remains the newest request for that path. Active-path ownership is separate:
 * a valid late response may warm its inactive tab but cannot select it. */
export function acceptWorkspaceSourceLoad(
  state: WorkspaceSourceLoadState,
  request: WorkspaceSourceLoadRequest,
): boolean {
  return request.generation === state.generation
    && state.latestRequestByPath.get(request.path) === request.requestId
}
