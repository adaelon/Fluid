import type { ReadingAnchor } from './api.ts'

export type CodeReadingAnchor = Extract<ReadingAnchor, { kind: 'code' }>

export const MAX_CODE_ANCHOR_OFFSET_PX = 1_000_000
export const MAX_CODE_ANCHOR_TOTAL_LINES = 100_000_000

export interface CodeAnchorCapture {
  topLine: number
  offsetPx: number
  totalLines: number
}

export interface ResolvedCodeReadingAnchor {
  lineNumber: number
  offsetPx: number
  lineCountChanged: boolean
}

export interface CodeAnchorScrollGeometry {
  scrollTop: number
  currentOffsetPx: number
  savedOffsetPx: number
  maxScrollTop: number
}

function normalizedLineCount(value: unknown): number | null {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 1
    && value <= MAX_CODE_ANCHOR_TOTAL_LINES
    ? value
    : null
}

function normalizedOffset(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null
  return Math.min(MAX_CODE_ANCHOR_OFFSET_PX, Math.max(-MAX_CODE_ANCHOR_OFFSET_PX, value))
}

/** Convert untrusted persisted/runtime data into the same bounded shape that
 * the backend accepts. Bad identities degrade to no anchor; finite offsets are
 * clamped so view geometry can never inject an unbounded persisted value. */
export function normalizeCodeReadingAnchor(value: unknown): CodeReadingAnchor | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as Record<string, unknown>
  if (candidate.kind !== 'code') return null

  const topLine = normalizedLineCount(candidate.topLine)
  const totalLines = normalizedLineCount(candidate.totalLines)
  const offsetPx = normalizedOffset(candidate.offsetPx)
  if (topLine === null || totalLines === null || offsetPx === null || topLine > totalLines) {
    return null
  }

  return { kind: 'code', topLine, offsetPx, totalLines }
}

/** Build a persisted code anchor from a measured CodeMirror viewport. */
export function captureCodeReadingAnchor(capture: CodeAnchorCapture): CodeReadingAnchor | null {
  return normalizeCodeReadingAnchor({ kind: 'code', ...capture })
}

/** Resolve a saved source line against the current document. A changed line
 * count keeps the same 1-based line when possible and otherwise clamps to EOF;
 * it deliberately never falls back to a stale pixel ratio. */
export function resolveCodeReadingAnchor(
  anchor: unknown,
  currentTotalLines: number,
): ResolvedCodeReadingAnchor | null {
  const normalized = normalizeCodeReadingAnchor(anchor)
  const currentLines = normalizedLineCount(currentTotalLines)
  if (!normalized || currentLines === null) return null

  return {
    lineNumber: Math.min(normalized.topLine, currentLines),
    offsetPx: normalized.offsetPx,
    lineCountChanged: normalized.totalLines !== currentLines,
  }
}

/** Translate measured line drift into one bounded scrollTop write. */
export function correctedCodeAnchorScrollTop(
  geometry: CodeAnchorScrollGeometry,
): number | null {
  const {
    scrollTop,
    currentOffsetPx,
    savedOffsetPx,
    maxScrollTop,
  } = geometry
  if (![scrollTop, currentOffsetPx, savedOffsetPx, maxScrollTop].every(Number.isFinite)) {
    return null
  }

  const extent = Math.max(0, maxScrollTop)
  const corrected = scrollTop + currentOffsetPx - savedOffsetPx
  return Math.min(extent, Math.max(0, corrected))
}
