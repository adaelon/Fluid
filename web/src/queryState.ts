import type {
  EvidenceStatus,
  QueryFrame,
  QueryPhase,
  SourceLink,
} from './ghostTypes'

export interface QueryEvidenceState {
  status: EvidenceStatus
  sources: SourceLink[]
  warning?: string
}

export type QueryViewMode = 'idle' | 'streaming' | 'done' | 'error'

export interface QueryViewState {
  mode: QueryViewMode
  phase: QueryPhase | 'connecting' | null
  statusMessage: string
  evidence: QueryEvidenceState | null
  answer: string
  errorMessage: string
}

export function idleQueryState(): QueryViewState {
  return {
    mode: 'idle',
    phase: null,
    statusMessage: '',
    evidence: null,
    answer: '',
    errorMessage: '',
  }
}

export function startQueryRequest(): QueryViewState {
  return {
    ...idleQueryState(),
    mode: 'streaming',
    phase: 'connecting',
    statusMessage: '正在连接追问服务',
  }
}

/** Pure projection of the query wire protocol into visible UI state. Evidence
 * and accumulated answer text survive every later status and terminal frame;
 * in particular, a stream error can be shown without erasing partial output. */
export function reduceQueryFrame(state: QueryViewState, frame: QueryFrame): QueryViewState {
  switch (frame.kind) {
    case 'status':
      return {
        ...state,
        mode: 'streaming',
        phase: frame.phase,
        statusMessage: frame.message,
        errorMessage: '',
      }
    case 'evidence':
      return {
        ...state,
        evidence: {
          status: frame.status,
          sources: [...(frame.sources ?? [])],
          warning: frame.warning,
        },
      }
    case 'delta':
      return {
        ...state,
        mode: 'streaming',
        answer: state.answer + frame.text,
      }
    case 'done':
      return {
        ...state,
        mode: 'done',
        phase: null,
        statusMessage: '',
      }
    case 'error':
      return {
        ...state,
        mode: 'error',
        phase: null,
        statusMessage: '',
        errorMessage: frame.message,
      }
  }
}
