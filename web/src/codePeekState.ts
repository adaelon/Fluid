import type { CodeEvidenceRef } from './ghostTypes'

export type CodePeekTarget = CodeEvidenceRef

export type CodePeekLoadState =
  | { mode: 'idle' }
  | { mode: 'loading'; requestId: number; target: CodePeekTarget }
  | {
      mode: 'ready'
      requestId: number
      target: CodePeekTarget
      source: string
      lang: string
    }
  | { mode: 'error'; requestId: number; target: CodePeekTarget; message: string }

export type CodePeekRangeValidation =
  | { ok: true; startLine: number; endLine: number }
  | { ok: false; message: string }

export function idleCodePeekState(): CodePeekLoadState {
  return { mode: 'idle' }
}

/** Match CodeMirror's string-document line semantics, including its final empty
 * line when source ends in a newline. */
export function codePeekDocumentLineCount(source: string): number {
  let lines = 1
  for (let index = 0; index < source.length; index++) {
    if (source.charCodeAt(index) === 10) lines++
  }
  return lines
}

/** Choose a stable whole source line for vertical centering of an inclusive range. */
export function codePeekCenterLine(target: CodePeekTarget): number {
  return Math.floor((target.startLine + target.endLine) / 2)
}

/** Validate backend evidence coordinates before any CodeMirror line lookup.
 * Query evidence is always 1-based and inclusive at both ends. */
export function validateCodePeekRange(
  target: CodePeekTarget,
  lineCount: number,
): CodePeekRangeValidation {
  if (!Number.isInteger(lineCount) || lineCount < 1) {
    return { ok: false, message: '无法校验代码证据范围：源码行数无效' }
  }
  if (
    !Number.isInteger(target.startLine) ||
    !Number.isInteger(target.endLine) ||
    target.startLine < 1 ||
    target.endLine < target.startLine
  ) {
    return {
      ok: false,
      message: `代码证据范围无效：${target.filePath}:${target.startLine}-${target.endLine} 必须是从 1 开始的闭区间`,
    }
  }
  if (target.endLine > lineCount) {
    return {
      ok: false,
      message: `代码证据范围越界：${target.filePath}:${target.startLine}-${target.endLine}，文件共 ${lineCount} 行`,
    }
  }
  return { ok: true, startLine: target.startLine, endLine: target.endLine }
}

export function startCodePeekRequest(
  requestId: number,
  target: CodePeekTarget,
): CodePeekLoadState {
  return { mode: 'loading', requestId, target }
}

/** Re-target the preview. A ready source is reused only for another valid range
 * in the same file; every other transition starts an isolated request. */
export function selectCodePeekTarget(
  state: CodePeekLoadState,
  target: CodePeekTarget,
  requestId: number,
): CodePeekLoadState {
  if (state.mode === 'ready' && state.target.filePath === target.filePath) {
    const range = validateCodePeekRange(target, codePeekDocumentLineCount(state.source))
    if (!range.ok) return { mode: 'error', requestId, target, message: range.message }
    return { ...state, target }
  }
  return startCodePeekRequest(requestId, target)
}

/** Commit a fetch result only to its matching live request. */
export function completeCodePeekRequest(
  state: CodePeekLoadState,
  requestId: number,
  source: string,
  lang: string,
): CodePeekLoadState {
  if (state.mode !== 'loading' || state.requestId !== requestId) return state
  const range = validateCodePeekRange(state.target, codePeekDocumentLineCount(source))
  if (!range.ok) {
    return {
      mode: 'error',
      requestId,
      target: state.target,
      message: range.message,
    }
  }
  return {
    mode: 'ready',
    requestId,
    target: state.target,
    source,
    lang,
  }
}

/** Commit a fetch failure only to its matching live request. */
export function failCodePeekRequest(
  state: CodePeekLoadState,
  requestId: number,
  message: string,
): CodePeekLoadState {
  if (state.mode !== 'loading' || state.requestId !== requestId) return state
  return {
    mode: 'error',
    requestId,
    target: state.target,
    message: message || '代码证据加载失败',
  }
}

/** Unmount/close moves the reducer to a state that rejects every late result. */
export function disposeCodePeekState(_state: CodePeekLoadState): CodePeekLoadState {
  return idleCodePeekState()
}
