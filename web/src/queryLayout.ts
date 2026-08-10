export const QUERY_DOCK_STORAGE_KEY = 'fluid:queryPanelPx'
export const QUERY_DOCK_DEFAULT_PX = 240
export const QUERY_DOCK_MIN_PX = 160
export const QUERY_DOCK_EDITOR_RESERVE_PX = 180
export const QUERY_DOCK_STATUS_BAR_PX = 24

export type SidebarView = 'hidden' | 'explorer' | 'query'
export type QueryPlacement = 'sidebar' | 'dock' | 'focus'
export type QueryFocusReturn = 'sidebar' | 'dock'
export type QueryPresentation = QueryPlacement

export interface QueryWorkspaceLayoutState {
  visible: boolean
  placement: QueryPlacement
  focusReturn: QueryFocusReturn
  sidebarView: SidebarView
}

export type QueryLayoutAction = {
  type:
    | 'activity-explorer'
    | 'activity-query'
    | 'status-toggle'
    | 'move-sidebar'
    | 'move-dock'
    | 'focus'
    | 'restore-focus'
    | 'escape'
    | 'close'
}

export function initialQueryWorkspaceLayout(): QueryWorkspaceLayoutState {
  return {
    visible: false,
    placement: 'dock',
    focusReturn: 'dock',
    sidebarView: 'explorer',
  }
}

export function queryWorkspaceLayoutValid(state: QueryWorkspaceLayoutState): boolean {
  if (!state.visible && state.placement === 'focus') return false
  if (!state.visible && state.sidebarView === 'query') return false
  if (state.placement !== 'focus' && state.focusReturn !== state.placement) return false
  if (state.visible && state.placement === 'sidebar') {
    return state.sidebarView === 'query'
  }
  if (state.sidebarView === 'query') {
    return state.visible
      && state.placement === 'focus'
      && state.focusReturn === 'sidebar'
  }
  if (state.visible && state.placement === 'focus') {
    return state.focusReturn === 'dock'
  }
  return true
}

function closeQueryLayout(
  state: QueryWorkspaceLayoutState,
): QueryWorkspaceLayoutState {
  const placement = state.placement === 'focus' ? state.focusReturn : state.placement
  return {
    visible: false,
    placement,
    focusReturn: placement,
    sidebarView: placement === 'sidebar'
      ? 'hidden'
      : state.sidebarView === 'query'
        ? 'hidden'
        : state.sidebarView,
  }
}

/**
 * Pure presentation reducer. It owns only where the single QueryPanel is
 * projected; query text, selected history and the active stream stay in the
 * project-scoped QueryWorkspaceController.
 */
export function reduceQueryWorkspaceLayout(
  state: QueryWorkspaceLayoutState,
  action: QueryLayoutAction,
): QueryWorkspaceLayoutState {
  switch (action.type) {
    case 'activity-explorer':
      if (state.placement === 'focus') return state
      if (state.sidebarView === 'query') {
        return {
          ...closeQueryLayout(state),
          sidebarView: 'explorer',
        }
      }
      return {
        ...state,
        sidebarView: state.sidebarView === 'explorer' ? 'hidden' : 'explorer',
      }
    case 'activity-query':
      if (state.placement === 'focus') return state
      if (state.visible && state.placement === 'sidebar') {
        return closeQueryLayout(state)
      }
      return {
        visible: true,
        placement: 'sidebar',
        focusReturn: 'sidebar',
        sidebarView: 'query',
      }
    case 'status-toggle':
      if (state.visible) return closeQueryLayout(state)
      return {
        visible: true,
        placement: 'dock',
        focusReturn: 'dock',
        sidebarView: state.sidebarView === 'query' ? 'hidden' : state.sidebarView,
      }
    case 'move-sidebar':
      if (state.placement === 'focus') return state
      return {
        visible: true,
        placement: 'sidebar',
        focusReturn: 'sidebar',
        sidebarView: 'query',
      }
    case 'move-dock':
      if (state.placement === 'focus') return state
      return {
        visible: true,
        placement: 'dock',
        focusReturn: 'dock',
        sidebarView: state.sidebarView === 'query' ? 'explorer' : state.sidebarView,
      }
    case 'focus':
      if (!state.visible || state.placement === 'focus') return state
      return {
        ...state,
        placement: 'focus',
        focusReturn: state.placement,
      }
    case 'restore-focus':
    case 'escape':
      if (state.placement !== 'focus') return state
      return {
        ...state,
        placement: state.focusReturn,
        sidebarView: state.focusReturn === 'sidebar'
          ? 'query'
          : state.sidebarView === 'query'
            ? 'explorer'
            : state.sidebarView,
      }
    case 'close':
      return closeQueryLayout(state)
  }
}

export const QUERY_PEEK_STORAGE_KEY = 'fluid:queryPeekPx'
export const QUERY_PEEK_DEFAULT_RATIO = 0.46
export const QUERY_PEEK_MIN_PX = 320
export const QUERY_PEEK_ANSWER_RESERVE_PX = 480
export const QUERY_PEEK_DIVIDER_PX = 6

export interface QueryDockHeightBounds {
  min: number
  max: number
}

export interface CodePeekWidthBounds {
  min: number
  max: number
}

function normalizedViewportHeight(viewportHeight: number): number {
  if (Number.isFinite(viewportHeight) && viewportHeight > 0) {
    return Math.round(viewportHeight)
  }
  return QUERY_DOCK_DEFAULT_PX + QUERY_DOCK_EDITOR_RESERVE_PX + QUERY_DOCK_STATUS_BAR_PX
}

function normalizedViewportWidth(viewportWidth: number): number {
  if (Number.isFinite(viewportWidth) && viewportWidth > 0) {
    return Math.round(viewportWidth)
  }
  return 1280
}

/**
 * Keep the editor body and status bar visible. On exceptionally short
 * viewports the editor reserve wins, so the dock may fall below its nominal
 * minimum instead of overflowing the shell.
 */
export function queryDockHeightBounds(viewportHeight: number): QueryDockHeightBounds {
  const max = Math.max(
    0,
    normalizedViewportHeight(viewportHeight) -
      QUERY_DOCK_EDITOR_RESERVE_PX -
      QUERY_DOCK_STATUS_BAR_PX,
  )
  return {
    min: Math.min(QUERY_DOCK_MIN_PX, max),
    max,
  }
}

export function clampQueryDockHeight(requestedPx: number, viewportHeight: number): number {
  const bounds = queryDockHeightBounds(viewportHeight)
  const fallback = Math.min(QUERY_DOCK_DEFAULT_PX, bounds.max)
  const requested = Number.isFinite(requestedPx) ? Math.round(requestedPx) : fallback
  return Math.min(bounds.max, Math.max(bounds.min, requested))
}

export function queryDockHeightFromPointer(
  startHeight: number,
  startPointerY: number,
  currentPointerY: number,
  viewportHeight: number,
): number {
  return clampQueryDockHeight(
    startHeight + startPointerY - currentPointerY,
    viewportHeight,
  )
}

/** Normalize persisted input without accepting partial numbers such as `240px`. */
export function loadQueryDockHeight(raw: string | null, viewportHeight: number): number {
  if (raw === null || raw.trim() === '') {
    return clampQueryDockHeight(QUERY_DOCK_DEFAULT_PX, viewportHeight)
  }
  const parsed = Number(raw)
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return clampQueryDockHeight(QUERY_DOCK_DEFAULT_PX, viewportHeight)
  }
  return clampQueryDockHeight(parsed, viewportHeight)
}

/** Keep the answer readable before honoring Code Peek's nominal minimum. */
export function codePeekWidthBounds(viewportWidth: number): CodePeekWidthBounds {
  const max = Math.max(
    0,
    normalizedViewportWidth(viewportWidth) -
      QUERY_PEEK_ANSWER_RESERVE_PX -
      QUERY_PEEK_DIVIDER_PX,
  )
  return {
    min: Math.min(QUERY_PEEK_MIN_PX, max),
    max,
  }
}

export function clampCodePeekWidth(requestedPx: number, viewportWidth: number): number {
  const normalizedWidth = normalizedViewportWidth(viewportWidth)
  const bounds = codePeekWidthBounds(normalizedWidth)
  const fallback = Math.round(normalizedWidth * QUERY_PEEK_DEFAULT_RATIO)
  const requested = Number.isFinite(requestedPx) ? Math.round(requestedPx) : fallback
  return Math.min(bounds.max, Math.max(bounds.min, requested))
}

export function codePeekWidthFromPointer(
  startWidth: number,
  startPointerX: number,
  currentPointerX: number,
  viewportWidth: number,
): number {
  return clampCodePeekWidth(
    startWidth + startPointerX - currentPointerX,
    viewportWidth,
  )
}

/** Normalize persisted input without accepting partial numbers such as `460px`. */
export function loadCodePeekWidth(raw: string | null, viewportWidth: number): number {
  if (raw === null || raw.trim() === '') {
    return clampCodePeekWidth(Number.NaN, viewportWidth)
  }
  const parsed = Number(raw)
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return clampCodePeekWidth(Number.NaN, viewportWidth)
  }
  return clampCodePeekWidth(parsed, viewportWidth)
}
