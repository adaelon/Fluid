import type {
  EvidenceStatus,
  QueryFrame,
  QueryPhase,
  QueryTrace,
  SourceLink,
} from './ghostTypes'

export interface QueryScopeIdentity {
  scopeKey: string
  scopeRevision: string
}

export function currentQueryScope(path: string, orientationId: string): QueryScopeIdentity {
  return {
    scopeKey: `current:${path}`,
    scopeRevision: orientationId,
  }
}

/** Canonical identity for the selected-file-set scope. S-QTRACE-1 has no source
 * digest endpoint yet, so this revision represents the normalized set snapshot;
 * the reducer intentionally accepts an independent revision field so a later
 * server-provided source identity can replace it without changing trace logic. */
export function selectedQueryScope(paths: string[]): QueryScopeIdentity {
  const normalized = Array.from(new Set(paths)).sort()
  const encoded = JSON.stringify(normalized)
  return {
    scopeKey: `selected:${encoded}`,
    scopeRevision: `selected-v1:${encoded}`,
  }
}

export function startQueryTrace(scope: QueryScopeIdentity, originalQuestion: string): QueryTrace {
  return {
    scopeKey: scope.scopeKey,
    scopeRevision: scope.scopeRevision,
    originalQuestion,
    turns: [],
  }
}

/** Keep a trace only while both its activation scope and revision still match. */
export function alignQueryTrace(
  trace: QueryTrace | null,
  scope: QueryScopeIdentity,
): QueryTrace | null {
  if (!trace) return null
  return trace.scopeKey === scope.scopeKey && trace.scopeRevision === scope.scopeRevision
    ? trace
    : null
}

/** Append only a terminally completed answer. The caller invokes this on `done`,
 * never on partial delta/error, so every stored turn remains replayable as a pair. */
export function appendCompletedQueryTurn(
  trace: QueryTrace | null,
  scope: QueryScopeIdentity,
  question: string,
  answer: string,
  codeEvidenceIds: string[] = [],
): QueryTrace {
  const base = alignQueryTrace(trace, scope) ?? startQueryTrace(scope, question)
  return {
    ...base,
    turns: [
      ...base.turns,
      {
        question,
        answer,
        codeEvidenceIds: Array.from(new Set(codeEvidenceIds)),
      },
    ],
  }
}

export interface QueryEvidenceState {
  status: EvidenceStatus
  sources: SourceLink[]
  warning?: string
}

export type QueryViewMode = 'idle' | 'streaming' | 'done' | 'error'

export interface QueryViewState {
  mode: QueryViewMode
  requestId: string | null
  phase: QueryPhase | 'connecting' | null
  statusMessage: string
  evidence: QueryEvidenceState | null
  answer: string
  errorMessage: string
}

export function idleQueryState(): QueryViewState {
  return {
    mode: 'idle',
    requestId: null,
    phase: null,
    statusMessage: '',
    evidence: null,
    answer: '',
    errorMessage: '',
  }
}

export function startQueryRequest(requestId: string | null = null): QueryViewState {
  return {
    ...idleQueryState(),
    mode: 'streaming',
    requestId,
    phase: 'connecting',
    statusMessage: '正在连接追问服务',
  }
}

/** Pure projection of the query wire protocol into visible UI state. Evidence
 * and accumulated answer text survive every later status and terminal frame;
 * in particular, a stream error can be shown without erasing partial output. */
export function reduceQueryFrame(state: QueryViewState, frame: QueryFrame): QueryViewState {
  if (state.requestId !== null && frame.reqId !== state.requestId) return state
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
