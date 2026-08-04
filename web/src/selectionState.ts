import type {
  SelectionExplanation,
  SelectionFrame,
  SelectionPhase,
} from './ghostTypes'

export interface SelectionByteRange {
  startByte: number
  endByte: number
  selectedText: string
}

export type SelectionViewState =
  | { mode: 'idle' }
  | {
      mode: 'loading'
      phase: SelectionPhase | 'connecting'
      message: string
      cacheHit: boolean
    }
  | {
      mode: 'result'
      explanation: SelectionExplanation
      cacheHit: boolean
    }
  | { mode: 'error'; message: string; cacheHit: boolean }

const encoder = new TextEncoder()

function splitsSurrogatePair(source: string, offset: number): boolean {
  if (offset <= 0 || offset >= source.length) return false
  const before = source.charCodeAt(offset - 1)
  const after = source.charCodeAt(offset)
  return before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff
}

/** Convert CodeMirror's UTF-16 document offsets to the backend's UTF-8 byte
 * range. Invalid, whitespace-only, multiline, or surrogate-splitting ranges
 * return null before any WebSocket is opened. */
export function selectionToUtf8ByteRange(
  source: string,
  from: number,
  to: number,
): SelectionByteRange | null {
  if (!Number.isInteger(from) || !Number.isInteger(to)) return null
  if (from < 0 || from >= to || to > source.length) return null
  if (splitsSurrogatePair(source, from) || splitsSurrogatePair(source, to)) return null

  const selectedText = source.slice(from, to)
  if (!selectedText.trim() || selectedText.includes('\n') || selectedText.includes('\r')) {
    return null
  }

  return {
    startByte: encoder.encode(source.slice(0, from)).length,
    endByte: encoder.encode(source.slice(0, to)).length,
    selectedText,
  }
}

export function startSelectionRequest(): SelectionViewState {
  return {
    mode: 'loading',
    phase: 'connecting',
    message: '正在连接解释服务',
    cacheHit: false,
  }
}

/** Pure state transition for the selection WebSocket. Frames for connection
 * failures are normalized to `error` by api.ts before they reach this reducer. */
export function reduceSelectionFrame(
  state: SelectionViewState,
  frame: SelectionFrame,
): SelectionViewState {
  switch (frame.kind) {
    case 'cache-hit':
      return state.mode === 'loading' ? { ...state, cacheHit: true } : state
    case 'status':
      return {
        mode: 'loading',
        phase: frame.phase,
        message: frame.message,
        cacheHit: state.mode === 'idle' ? false : state.cacheHit,
      }
    case 'result':
      return {
        mode: 'result',
        explanation: frame.explanation,
        cacheHit: state.mode === 'idle' ? false : state.cacheHit,
      }
    case 'error':
      return {
        mode: 'error',
        message: frame.message,
        cacheHit: state.mode === 'idle' ? false : state.cacheHit,
      }
    case 'done':
      return state
  }
}
