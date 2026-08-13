import type { ProjectReadingSnapshot } from './api.ts'

export type WorkspacePhase = 'booting' | 'restoring' | 'ready' | 'switching'
export type WorkspaceSnapshotChange = 'structure' | 'scroll'

export const WORKSPACE_STRUCTURE_DEBOUNCE_MS = 150
export const WORKSPACE_SCROLL_DEBOUNCE_MS = 400

export interface WorkspaceNotice {
  kind: 'warning' | 'error'
  message: string
}

export interface WorkspaceRestoreIssues {
  backendWarnings: number
  skippedDirectories: number
  skippedOpenFiles: number
  skippedReadingPositions: number
}

export interface WorkspaceTimerDriver {
  setTimeout(callback: () => void, delayMs: number): unknown
  clearTimeout(handle: unknown): void
}

export interface WorkspaceControllerDependencies {
  save(request: {
    projectRoot: string
    snapshot: ProjectReadingSnapshot
  }): Promise<{ saved: true }>
  timers?: WorkspaceTimerDriver
  notify?: (notice: WorkspaceNotice) => void
  onPhaseChange?: (phase: WorkspacePhase) => void
}

interface WorkspaceGenerationState {
  generation: number
  projectRoot: string | null
  snapshot: ProjectReadingSnapshot
  revision: number
  savedRevision: number
  lastAttemptedRevision: number
  drainRequested: boolean
  structureTimer: unknown | null
  scrollTimer: unknown | null
}

interface WorkspaceSaveFlight {
  state: WorkspaceGenerationState
  revision: number
  promise: Promise<void>
}

const defaultTimers: WorkspaceTimerDriver = {
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) => {
    globalThis.clearTimeout(handle as ReturnType<typeof globalThis.setTimeout>)
  },
}

function emptySnapshot(): ProjectReadingSnapshot {
  return {
    expandedDirectories: [],
    openFiles: [],
    activeFile: null,
    readingPositions: {},
  }
}

function cloneSnapshot(snapshot: ProjectReadingSnapshot): ProjectReadingSnapshot {
  return {
    expandedDirectories: [...snapshot.expandedDirectories],
    openFiles: [...snapshot.openFiles],
    activeFile: snapshot.activeFile,
    readingPositions: Object.fromEntries(
      Object.entries(snapshot.readingPositions).map(([path, anchor]) => [path, { ...anchor }]),
    ),
  }
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index])
}

function sameAnchor(
  left: ProjectReadingSnapshot['readingPositions'][string],
  right: ProjectReadingSnapshot['readingPositions'][string],
): boolean {
  if (left.kind !== right.kind) return false
  if (left.kind === 'code' && right.kind === 'code') {
    return left.topLine === right.topLine
      && left.offsetPx === right.offsetPx
      && left.totalLines === right.totalLines
  }
  if (left.kind === 'markdown' && right.kind === 'markdown') {
    return left.blockDigest === right.blockDigest
      && left.occurrence === right.occurrence
      && left.offsetPx === right.offsetPx
      && left.scrollRatio === right.scrollRatio
  }
  return false
}

function sameSnapshot(
  left: ProjectReadingSnapshot,
  right: ProjectReadingSnapshot,
): boolean {
  if (
    left.activeFile !== right.activeFile
    || !sameStrings(left.expandedDirectories, right.expandedDirectories)
    || !sameStrings(left.openFiles, right.openFiles)
  ) return false

  const leftPaths = Object.keys(left.readingPositions)
  const rightPaths = Object.keys(right.readingPositions)
  return leftPaths.length === rightPaths.length && leftPaths.every((path) => {
    const rightAnchor = right.readingPositions[path]
    return rightAnchor !== undefined
      && sameAnchor(left.readingPositions[path], rightAnchor)
  })
}

function restoreNoticeMessage(issues: WorkspaceRestoreIssues): string | null {
  const parts = [
    issues.backendWarnings > 0 ? `记录 ${issues.backendWarnings} 项告警` : null,
    issues.skippedDirectories > 0 ? `目录 ${issues.skippedDirectories} 项` : null,
    issues.skippedOpenFiles > 0 ? `标签 ${issues.skippedOpenFiles} 项` : null,
    issues.skippedReadingPositions > 0
      ? `阅读位置 ${issues.skippedReadingPositions} 项`
      : null,
  ].filter((part): part is string => part !== null)
  return parts.length > 0 ? `阅读现场已部分恢复：${parts.join('、')}。` : null
}

export class WorkspaceController {
  private readonly timers: WorkspaceTimerDriver
  private active: WorkspaceGenerationState = {
    generation: 0,
    projectRoot: null,
    snapshot: emptySnapshot(),
    revision: 0,
    savedRevision: 0,
    lastAttemptedRevision: 0,
    drainRequested: false,
    structureTimer: null,
    scrollTimer: null,
  }
  private currentPhase: WorkspacePhase = 'booting'
  private saveFlight: WorkspaceSaveFlight | null = null
  private restoreNoticeGeneration = -1
  private readonly saveFailureNotices = new Set<string>()

  constructor(private readonly dependencies: WorkspaceControllerDependencies) {
    this.timers = dependencies.timers ?? defaultTimers
  }

  get phase(): WorkspacePhase {
    return this.currentPhase
  }

  get projectRoot(): string | null {
    return this.active.projectRoot
  }

  get snapshot(): ProjectReadingSnapshot {
    return cloneSnapshot(this.active.snapshot)
  }

  get dirty(): boolean {
    return this.active.revision > this.active.savedRevision
  }

  beginRestore(projectRoot: string | null, snapshot: ProjectReadingSnapshot): void {
    this.cancelTimer(this.active)
    this.active = {
      generation: this.active.generation + 1,
      projectRoot,
      snapshot: cloneSnapshot(snapshot),
      revision: 0,
      savedRevision: 0,
      lastAttemptedRevision: 0,
      drainRequested: false,
      structureTimer: null,
      scrollTimer: null,
    }
    this.setPhase('restoring')
  }

  completeRestore(): void {
    if (this.currentPhase === 'restoring' || this.currentPhase === 'booting') {
      this.setPhase('ready')
    }
  }

  beginSwitch(): void {
    this.cancelTimer(this.active)
    this.setPhase('switching')
  }

  cancelSwitch(): void {
    if (this.currentPhase === 'switching') this.setPhase('ready')
  }

  updateSnapshot(snapshot: ProjectReadingSnapshot, change: WorkspaceSnapshotChange): boolean {
    const next = cloneSnapshot(snapshot)
    if (sameSnapshot(this.active.snapshot, next)) return false
    this.active.snapshot = next

    // Restoration owns a progressively assembled in-memory projection. It is
    // intentionally clean: no watcher or programmatic scroll may persist a
    // half-restored or merely sanitized record.
    if (this.currentPhase !== 'ready' || this.active.projectRoot === null) return true

    this.active.revision++
    this.scheduleSave(this.active, change)
    return true
  }

  reportRestoreIssues(issues: WorkspaceRestoreIssues): void {
    if (this.restoreNoticeGeneration === this.active.generation) return
    const message = restoreNoticeMessage(issues)
    if (!message) return
    this.restoreNoticeGeneration = this.active.generation
    this.dependencies.notify?.({ kind: 'warning', message })
  }

  /** Commit the latest whole snapshot now. Failures are absorbed after a
   * single notice so a project switch and page teardown remain best-effort. */
  async flush(): Promise<void> {
    const state = this.active
    this.cancelTimer(state)
    if (state.projectRoot === null || state.revision <= state.savedRevision) return
    state.drainRequested = true
    await this.drain(state, true)
  }

  cancelScheduledSave(): void {
    this.cancelTimer(this.active)
  }

  private setPhase(phase: WorkspacePhase): void {
    if (this.currentPhase === phase) return
    this.currentPhase = phase
    this.dependencies.onPhaseChange?.(phase)
  }

  private scheduleSave(
    state: WorkspaceGenerationState,
    change: WorkspaceSnapshotChange,
  ): void {
    const timerKey = change === 'scroll' ? 'scrollTimer' : 'structureTimer'
    const delayMs = change === 'scroll'
      ? WORKSPACE_SCROLL_DEBOUNCE_MS
      : WORKSPACE_STRUCTURE_DEBOUNCE_MS
    const current = state[timerKey]
    if (current !== null) this.timers.clearTimeout(current)
    state[timerKey] = this.timers.setTimeout(() => {
      state[timerKey] = null
      // Either debounce channel commits the latest whole snapshot, so its
      // sibling timer would only repeat bytes that are already represented.
      this.cancelTimer(state)
      state.drainRequested = true
      void this.drain(state, false)
    }, delayMs)
  }

  private cancelTimer(state: WorkspaceGenerationState): void {
    if (state.structureTimer !== null) {
      this.timers.clearTimeout(state.structureTimer)
      state.structureTimer = null
    }
    if (state.scrollTimer !== null) {
      this.timers.clearTimeout(state.scrollTimer)
      state.scrollTimer = null
    }
  }

  private async drain(state: WorkspaceGenerationState, force: boolean): Promise<void> {
    if (state.projectRoot === null || state.revision <= state.savedRevision) {
      state.drainRequested = false
      return
    }

    const existing = this.saveFlight
    if (existing) {
      state.drainRequested = true
      await existing.promise
      if (state.revision <= state.savedRevision) {
        state.drainRequested = false
        return
      }
      await this.drain(state, force)
      return
    }

    if (!force && !state.drainRequested) return
    if (!force && state.revision <= state.lastAttemptedRevision) {
      state.drainRequested = false
      return
    }

    const revision = state.revision
    const projectRoot = state.projectRoot
    const request = {
      projectRoot,
      snapshot: cloneSnapshot(state.snapshot),
    }
    state.lastAttemptedRevision = revision
    state.drainRequested = false

    const promise = this.dependencies.save(request).then(
      () => {
        state.savedRevision = Math.max(state.savedRevision, revision)
      },
      (error: unknown) => {
        // A superseded project save may legitimately lose the backend root race
        // and receive 409. Its original generation stays dirty for bookkeeping,
        // but the new root owns user-visible retry state and notifications.
        if (
          this.active !== state
          && error instanceof Error
          && 'status' in error
          && error.status === 409
        ) return
        if (this.active !== state || this.saveFailureNotices.has(projectRoot)) return
        this.saveFailureNotices.add(projectRoot)
        this.dependencies.notify?.({
          kind: 'error',
          message: '阅读现场保存失败；后续更改时会自动重试。',
        })
      },
    )
    this.saveFlight = { state, revision, promise }
    await promise
    if (this.saveFlight?.promise === promise) this.saveFlight = null

    if (
      state.drainRequested
      && state.revision > state.savedRevision
      && state.revision > state.lastAttemptedRevision
    ) {
      await this.drain(state, false)
    }
  }
}

export function createWorkspaceController(
  dependencies: WorkspaceControllerDependencies,
): WorkspaceController {
  return new WorkspaceController(dependencies)
}
