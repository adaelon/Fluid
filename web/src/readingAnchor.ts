import type { ReadingAnchor } from './api.ts'

export type CodeReadingAnchor = Extract<ReadingAnchor, { kind: 'code' }>
export type MarkdownReadingAnchor = Extract<ReadingAnchor, { kind: 'markdown' }>

export const MAX_CODE_ANCHOR_OFFSET_PX = 1_000_000
export const MAX_CODE_ANCHOR_TOTAL_LINES = 100_000_000
export const MAX_MARKDOWN_ANCHOR_OFFSET_PX = 1_000_000
export const MAX_MARKDOWN_ANCHOR_OCCURRENCE = 10_000_000
export const MAX_MARKDOWN_BLOCK_DIGEST_BYTES = 256

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

export interface MarkdownBlockIdentity {
  blockDigest: string
  occurrence: number
}

export interface MarkdownAnchorCapture extends MarkdownBlockIdentity {
  offsetPx: number
  scrollTop: number
  scrollHeight: number
  clientHeight: number
}

export type ResolvedMarkdownReadingAnchor =
  | {
      mode: 'block'
      blockIndex: number
      offsetPx: number
    }
  | {
      mode: 'ratio'
      scrollRatio: number
    }

export interface MarkdownAnchorScrollGeometry {
  scrollTop: number
  currentOffsetPx: number | null
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

function normalizedOccurrence(value: unknown): number | null {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 0
    && value <= MAX_MARKDOWN_ANCHOR_OCCURRENCE
    ? value
    : null
}

function normalizedScrollRatio(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null
  return Math.min(1, Math.max(0, value))
}

function isControlCharacter(character: string): boolean {
  const codePoint = character.codePointAt(0)
  return codePoint !== undefined
    && (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f))
}

function validMarkdownBlockDigest(value: unknown): value is string {
  if (typeof value !== 'string' || value.length === 0) return false
  if (new TextEncoder().encode(value).byteLength > MAX_MARKDOWN_BLOCK_DIGEST_BYTES) return false
  return Array.from(value).every((character) => (
    !isControlCharacter(character) && !/\s/u.test(character)
  ))
}

function rotateRight(value: number, distance: number): number {
  return (value >>> distance) | (value << (32 - distance))
}

const SHA256_ROUND_CONSTANTS = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
  0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
  0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
  0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
  0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
])

/** A small synchronous SHA-256 implementation keeps capture synchronous: tab
 * switches must be able to snapshot the current document before changing the
 * active path, while Web Crypto exposes digest only as an async operation. */
function sha256Utf8(value: string): string {
  const bytes = new TextEncoder().encode(value)
  const paddedLength = Math.ceil((bytes.length + 9) / 64) * 64
  const padded = new Uint8Array(paddedLength)
  padded.set(bytes)
  padded[bytes.length] = 0x80

  const bitLength = bytes.length * 8
  const data = new DataView(padded.buffer)
  data.setUint32(paddedLength - 8, Math.floor(bitLength / 0x1_0000_0000))
  data.setUint32(paddedLength - 4, bitLength >>> 0)

  const hash = new Uint32Array([
    0x6a09e667,
    0xbb67ae85,
    0x3c6ef372,
    0xa54ff53a,
    0x510e527f,
    0x9b05688c,
    0x1f83d9ab,
    0x5be0cd19,
  ])
  const words = new Uint32Array(64)

  for (let chunk = 0; chunk < paddedLength; chunk += 64) {
    for (let index = 0; index < 16; index++) {
      words[index] = data.getUint32(chunk + index * 4)
    }
    for (let index = 16; index < 64; index++) {
      const previous = words[index - 15]
      const earlier = words[index - 2]
      const sigma0 = rotateRight(previous, 7) ^ rotateRight(previous, 18) ^ (previous >>> 3)
      const sigma1 = rotateRight(earlier, 17) ^ rotateRight(earlier, 19) ^ (earlier >>> 10)
      words[index] = (words[index - 16] + sigma0 + words[index - 7] + sigma1) >>> 0
    }

    let a = hash[0]
    let b = hash[1]
    let c = hash[2]
    let d = hash[3]
    let e = hash[4]
    let f = hash[5]
    let g = hash[6]
    let h = hash[7]

    for (let index = 0; index < 64; index++) {
      const sum1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25)
      const choose = (e & f) ^ (~e & g)
      const temporary1 = (h + sum1 + choose + SHA256_ROUND_CONSTANTS[index] + words[index]) >>> 0
      const sum0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22)
      const majority = (a & b) ^ (a & c) ^ (b & c)
      const temporary2 = (sum0 + majority) >>> 0
      h = g
      g = f
      f = e
      e = (d + temporary1) >>> 0
      d = c
      c = b
      b = a
      a = (temporary1 + temporary2) >>> 0
    }

    hash[0] = (hash[0] + a) >>> 0
    hash[1] = (hash[1] + b) >>> 0
    hash[2] = (hash[2] + c) >>> 0
    hash[3] = (hash[3] + d) >>> 0
    hash[4] = (hash[4] + e) >>> 0
    hash[5] = (hash[5] + f) >>> 0
    hash[6] = (hash[6] + g) >>> 0
    hash[7] = (hash[7] + h) >>> 0
  }

  return Array.from(hash, (word) => word.toString(16).padStart(8, '0')).join('')
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

/** Normalize rendered block text into a layout-independent identity input. */
export function normalizeMarkdownBlockText(value: string): string {
  return value.normalize('NFKC').replace(/\s+/gu, ' ').trim()
}

/** Hash normalized visible text without retaining document content in state. */
export function digestMarkdownBlockText(value: string): string | null {
  const normalized = normalizeMarkdownBlockText(value)
  return normalized.length > 0 ? sha256Utf8(normalized) : null
}

/** Assign each non-empty block its digest-local, zero-based occurrence. */
export function indexMarkdownContentBlocks(
  visibleTexts: readonly string[],
): Array<MarkdownBlockIdentity | null> {
  const nextOccurrenceByDigest = new Map<string, number>()
  return visibleTexts.map((text) => {
    const blockDigest = digestMarkdownBlockText(text)
    if (!blockDigest) return null
    const occurrence = nextOccurrenceByDigest.get(blockDigest) ?? 0
    nextOccurrenceByDigest.set(blockDigest, occurrence + 1)
    return occurrence <= MAX_MARKDOWN_ANCHOR_OCCURRENCE
      ? { blockDigest, occurrence }
      : null
  })
}

/** Convert untrusted persisted/runtime Markdown data into a bounded anchor. */
export function normalizeMarkdownReadingAnchor(value: unknown): MarkdownReadingAnchor | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as Record<string, unknown>
  if (candidate.kind !== 'markdown') return null

  const blockDigest = candidate.blockDigest
  const occurrence = normalizedOccurrence(candidate.occurrence)
  const offsetPx = normalizedOffset(candidate.offsetPx)
  const scrollRatio = normalizedScrollRatio(candidate.scrollRatio)
  if (
    !validMarkdownBlockDigest(blockDigest)
    || occurrence === null
    || offsetPx === null
    || scrollRatio === null
  ) {
    return null
  }

  return { kind: 'markdown', blockDigest, occurrence, offsetPx, scrollRatio }
}

/** Build a Markdown anchor from one measured content block and scroll extent. */
export function captureMarkdownReadingAnchor(
  capture: MarkdownAnchorCapture,
): MarkdownReadingAnchor | null {
  const { scrollTop, scrollHeight, clientHeight, ...identity } = capture
  if (![scrollTop, scrollHeight, clientHeight].every(Number.isFinite)) return null
  const maxScrollTop = Math.max(1, scrollHeight - clientHeight)
  return normalizeMarkdownReadingAnchor({
    kind: 'markdown',
    ...identity,
    scrollRatio: Math.min(1, Math.max(0, scrollTop / maxScrollTop)),
  })
}

/** Prefer the exact digest occurrence; use ratio only when content disappeared. */
export function resolveMarkdownReadingAnchor(
  anchor: unknown,
  currentBlocks: readonly (MarkdownBlockIdentity | null)[],
): ResolvedMarkdownReadingAnchor | null {
  const normalized = normalizeMarkdownReadingAnchor(anchor)
  if (!normalized) return null

  const blockIndex = currentBlocks.findIndex((block) => (
    block?.blockDigest === normalized.blockDigest
    && block.occurrence === normalized.occurrence
  ))
  return blockIndex >= 0
    ? { mode: 'block', blockIndex, offsetPx: normalized.offsetPx }
    : { mode: 'ratio', scrollRatio: normalized.scrollRatio }
}

/** Resolve either current block drift or ratio fallback to one bounded write. */
export function correctedMarkdownAnchorScrollTop(
  resolved: ResolvedMarkdownReadingAnchor | null,
  geometry: MarkdownAnchorScrollGeometry,
): number | null {
  if (!resolved || !Number.isFinite(geometry.maxScrollTop)) return null
  const extent = Math.max(0, geometry.maxScrollTop)
  if (resolved.mode === 'ratio') return resolved.scrollRatio * extent
  if (!Number.isFinite(geometry.scrollTop) || !Number.isFinite(geometry.currentOffsetPx)) {
    return null
  }

  const corrected = geometry.scrollTop
    + (geometry.currentOffsetPx as number)
    - resolved.offsetPx
  return Math.min(extent, Math.max(0, corrected))
}
