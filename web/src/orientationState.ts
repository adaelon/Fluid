import type {
  FileOrientationCard,
  OrientationFrame,
  OrientationPhase,
} from './ghostTypes'

export type OrientationViewMode = 'idle' | 'loading' | 'ready' | 'error'

export interface OrientationViewState {
  mode: OrientationViewMode
  reqId: string | null
  filePath: string
  phase: OrientationPhase | 'connecting' | null
  message: string
  cacheHit: boolean
  card: FileOrientationCard | null
  errorMessage: string
}

export function idleOrientationState(): OrientationViewState {
  return {
    mode: 'idle',
    reqId: null,
    filePath: '',
    phase: null,
    message: '',
    cacheHit: false,
    card: null,
    errorMessage: '',
  }
}

export function startOrientationRequest(reqId: string, filePath: string): OrientationViewState {
  return {
    mode: 'loading',
    reqId,
    filePath,
    phase: 'connecting',
    message: '正在连接文件定向服务',
    cacheHit: false,
    card: null,
    errorMessage: '',
  }
}

/** Pure projection of the orientation wire protocol. Terminal states are
 * immutable; stale reqIds are returned by identity so rapid file switches
 * cannot write a prior card or error into the active file. */
export function reduceOrientationFrame(
  state: OrientationViewState,
  frame: OrientationFrame,
): OrientationViewState {
  if (state.mode !== 'loading' || frame.reqId !== state.reqId) return state

  switch (frame.kind) {
    case 'cache-hit':
      return { ...state, cacheHit: true }
    case 'status':
      return {
        ...state,
        phase: frame.phase,
        message: frame.message,
        errorMessage: '',
      }
    case 'card':
      if (frame.card.filePath !== state.filePath) {
        return {
          ...state,
          mode: 'error',
          phase: null,
          message: '',
          card: null,
          errorMessage: '定向卡与当前文件不匹配，请重试',
        }
      }
      return {
        ...state,
        card: frame.card,
        message: '定向卡已接收，正在完成激活检查',
      }
    case 'done':
      if (!state.card) {
        return {
          ...state,
          mode: 'error',
          phase: null,
          message: '',
          errorMessage: '定向完成但未收到文件定向卡，请重试',
        }
      }
      return {
        ...state,
        mode: 'ready',
        phase: null,
        message: '',
        errorMessage: '',
      }
    case 'error':
      return {
        ...state,
        mode: 'error',
        phase: null,
        message: '',
        errorMessage: frame.message,
      }
  }
}

/** The sole frontend gate for starting GenerationScheduler. */
export function orientationCanActivate(
  state: OrientationViewState,
  filePath: string,
): state is OrientationViewState & { mode: 'ready'; card: FileOrientationCard } {
  return (
    state.mode === 'ready' &&
    state.filePath === filePath &&
    state.card?.filePath === filePath
  )
}
