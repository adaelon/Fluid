import { Text as CmText } from '@codemirror/state'
import type { FindRange } from './inFileFind.ts'

export interface RenderedTextSegment {
  node: globalThis.Text
  from: number
  to: number
}

export interface RenderedTextIndex {
  text: CmText
  segments: RenderedTextSegment[]
}

export const MARKDOWN_FIND_ALL_HIGHLIGHT = 'fluid-markdown-find-match'
export const MARKDOWN_FIND_CURRENT_HIGHLIGHT = 'fluid-markdown-find-current'

const ELEMENT_NODE = 1
const TEXT_NODE = 3

const BLOCK_TAGS = new Set([
  'ADDRESS',
  'ARTICLE',
  'ASIDE',
  'BLOCKQUOTE',
  'DD',
  'DETAILS',
  'DIALOG',
  'DIV',
  'DL',
  'DT',
  'FIELDSET',
  'FIGCAPTION',
  'FIGURE',
  'FOOTER',
  'FORM',
  'H1',
  'H2',
  'H3',
  'H4',
  'H5',
  'H6',
  'HEADER',
  'HGROUP',
  'HR',
  'LI',
  'MAIN',
  'NAV',
  'OL',
  'P',
  'PRE',
  'SECTION',
  'SUMMARY',
  'TABLE',
  'TBODY',
  'TD',
  'TFOOT',
  'TH',
  'THEAD',
  'TR',
  'UL',
])

const STRUCTURAL_WHITESPACE_PARENTS = new Set([
  'ARTICLE',
  'ASIDE',
  'BLOCKQUOTE',
  'DIV',
  'DL',
  'FIELDSET',
  'FIGURE',
  'FOOTER',
  'FORM',
  'HEADER',
  'HGROUP',
  'MAIN',
  'NAV',
  'OL',
  'SECTION',
  'TABLE',
  'TBODY',
  'TFOOT',
  'THEAD',
  'TR',
  'UL',
])

const NON_TEXT_TAGS = new Set(['NOSCRIPT', 'SCRIPT', 'STYLE', 'TEMPLATE'])
const AUXILIARY_CLASSES = new Set([
  'katex-mathml',
  'MathJax_Preview',
  'mjx-assistive-mml',
  'screen-reader-only',
  'sr-only',
  'visually-hidden',
])

function tagName(element: Element): string {
  return element.tagName.toUpperCase()
}

function hasClass(element: Element, name: string): boolean {
  return element.classList.contains(name)
}

function isVisibleKatexBranch(element: Element): boolean {
  return hasClass(element, 'katex-html')
}

function isIgnoredElement(element: Element): boolean {
  if (NON_TEXT_TAGS.has(tagName(element))) return true
  if (element.hasAttribute('data-fluid-find-ignore')) return true
  if (Array.from(AUXILIARY_CLASSES).some((name) => hasClass(element, name))) return true

  const htmlElement = element as HTMLElement
  if (htmlElement.hidden || element.hasAttribute('hidden')) return true
  if (element.getAttribute('aria-hidden') === 'true' && !isVisibleKatexBranch(element)) {
    return true
  }

  const getStyle = element.ownerDocument.defaultView?.getComputedStyle
  if (!getStyle) return false
  const style = getStyle.call(element.ownerDocument.defaultView, element)
  if (style.display === 'none') return true
  if (style.visibility === 'hidden' || style.visibility === 'collapse') return true
  if (style.getPropertyValue('content-visibility') === 'hidden') return true
  return Number.parseFloat(style.opacity) === 0
}

function isStructuralWhitespace(node: globalThis.Text): boolean {
  if (!/^\s*$/.test(node.data)) return false
  const parent = node.parentElement
  return !parent || STRUCTURAL_WHITESPACE_PARENTS.has(tagName(parent))
}

/**
 * Flatten the rendered document's visible text into CodeMirror text while retaining
 * exact UTF-16 offsets back to the original DOM text nodes. Block boundaries and
 * line breaks become one stable newline, but never create a synthetic segment.
 */
export function buildRenderedTextIndex(root: HTMLElement): RenderedTextIndex {
  const parts: string[] = []
  const segments: RenderedTextSegment[] = []
  let length = 0
  let pendingBoundary = false
  let lastCharacter = ''

  function markBoundary(): void {
    if (length > 0) pendingBoundary = true
  }

  function appendText(node: globalThis.Text): void {
    const value = node.data
    if (!value || isStructuralWhitespace(node)) return

    if (pendingBoundary) {
      if (lastCharacter !== '\n' && !value.startsWith('\n')) {
        parts.push('\n')
        length++
        lastCharacter = '\n'
      }
      pendingBoundary = false
    }

    const from = length
    parts.push(value)
    length += value.length
    lastCharacter = value.at(-1) ?? lastCharacter
    segments.push({ node, from, to: length })
  }

  function visit(node: Node): void {
    if (node.nodeType === TEXT_NODE) {
      appendText(node as globalThis.Text)
      return
    }
    if (node.nodeType !== ELEMENT_NODE && node !== root) {
      for (const child of Array.from(node.childNodes)) visit(child)
      return
    }

    const element = node as Element
    if (element !== root && isIgnoredElement(element)) return

    const tag = tagName(element)
    if (tag === 'BR') {
      markBoundary()
      return
    }

    const block = element !== root && BLOCK_TAGS.has(tag)
    if (block) markBoundary()
    const segmentsBefore = segments.length
    for (const child of Array.from(node.childNodes)) visit(child)
    if (block && segments.length > segmentsBefore) markBoundary()
  }

  visit(root)
  const source = parts.join('')
  return {
    text: CmText.of(source.split('\n')),
    segments,
  }
}

interface DomPoint {
  node: globalThis.Text
  offset: number
}

function pointInsideSegment(
  index: RenderedTextIndex,
  offset: number,
): DomPoint | null {
  const segment = index.segments.find((candidate) => (
    candidate.from < offset && offset < candidate.to
  ))
  return segment ? { node: segment.node, offset: offset - segment.from } : null
}

function pointAtOffset(
  index: RenderedTextIndex,
  offset: number,
  edge: 'start' | 'end',
): DomPoint | null {
  const inside = pointInsideSegment(index, offset)
  if (inside) return inside

  const starts = index.segments.find((segment) => segment.from === offset)
  const ends = [...index.segments].reverse().find((segment) => segment.to === offset)
  if (edge === 'start') {
    if (starts) return { node: starts.node, offset: 0 }
    if (ends) return { node: ends.node, offset: ends.to - ends.from }
  } else {
    if (ends) return { node: ends.node, offset: ends.to - ends.from }
    if (starts) return { node: starts.node, offset: 0 }
  }

  const previous = [...index.segments].reverse().find((segment) => segment.to < offset)
  const next = index.segments.find((segment) => segment.from > offset)
  if (edge === 'start' && previous) {
    return { node: previous.node, offset: previous.to - previous.from }
  }
  if (edge === 'end' && next) return { node: next.node, offset: 0 }
  if (previous) return { node: previous.node, offset: previous.to - previous.from }
  return next ? { node: next.node, offset: 0 } : null
}

function collapsedPoint(index: RenderedTextIndex, offset: number): DomPoint | null {
  return pointAtOffset(index, offset, 'start') ?? pointAtOffset(index, offset, 'end')
}

/** Map one shared UTF-16 match back to a live DOM Range without rewriting HTML. */
export function renderedRangeForMatch(
  index: RenderedTextIndex,
  match: FindRange,
): Range | null {
  if (
    !Number.isInteger(match.from)
    || !Number.isInteger(match.to)
    || match.from < 0
    || match.to < match.from
    || match.to > index.text.length
  ) return null

  const start = match.from === match.to
    ? collapsedPoint(index, match.from)
    : pointAtOffset(index, match.from, 'start')
  const end = match.from === match.to
    ? start
    : pointAtOffset(index, match.to, 'end')
  if (!start || !end || start.node.ownerDocument !== end.node.ownerDocument) return null

  const range = start.node.ownerDocument.createRange()
  try {
    range.setStart(start.node, start.offset)
    range.setEnd(end.node, end.offset)
  } catch {
    range.detach()
    return null
  }
  return range
}

interface HighlightRegistryLike {
  set(name: string, highlight: Highlight): void
  delete(name: string): boolean
}

type HighlightFactory = (ranges: Range[]) => Highlight

export interface MappedMarkdownFindHighlights {
  all: Range[]
  current: Range | null
}

export interface RenderedHighlightRect {
  left: number
  top: number
  width: number
  height: number
  current: boolean
}

function browserHighlightRegistry(): HighlightRegistryLike | null {
  if (
    typeof CSS === 'undefined'
    || typeof Highlight === 'undefined'
    || !('highlights' in CSS)
  ) return null
  const registry = CSS.highlights
  return registry
    && typeof registry.set === 'function'
    && typeof registry.delete === 'function'
    ? registry
    : null
}

function browserHighlightFactory(ranges: Range[]): Highlight {
  return new Highlight(...ranges)
}

/**
 * Own the two CSS Custom Highlight registrations for one Markdown surface.
 * Starting a revision deletes every old Range before new DOM can be registered;
 * late work carrying an older revision can therefore never restore stale nodes.
 */
export class MarkdownFindHighlightLayer {
  private revision = 0
  private readonly registry: HighlightRegistryLike | null
  private readonly createHighlight: HighlightFactory

  constructor(
    registry: HighlightRegistryLike | null = browserHighlightRegistry(),
    createHighlight: HighlightFactory = browserHighlightFactory,
  ) {
    this.registry = registry
    this.createHighlight = createHighlight
  }

  beginRevision(): number {
    this.revision++
    this.clear()
    return this.revision
  }

  currentRevision(): number {
    return this.revision
  }

  usesNativeHighlights(): boolean {
    return this.registry !== null
  }

  clear(): void {
    this.registry?.delete(MARKDOWN_FIND_CURRENT_HIGHLIGHT)
    this.registry?.delete(MARKDOWN_FIND_ALL_HIGHLIGHT)
  }

  apply(
    revision: number,
    index: RenderedTextIndex,
    matches: readonly FindRange[],
    current: number,
  ): MappedMarkdownFindHighlights | null {
    if (revision !== this.revision) return null

    const mapped = matches.map((match) => renderedRangeForMatch(index, match))
    const all = mapped.filter((range): range is Range => range !== null)
    const active = current >= 1 && current <= mapped.length
      ? mapped[current - 1]
      : null

    if (revision !== this.revision) return null
    this.clear()
    if (this.registry && all.length > 0) {
      this.registry.set(MARKDOWN_FIND_ALL_HIGHLIGHT, this.createHighlight(all))
    }
    if (this.registry && active) {
      this.registry.set(MARKDOWN_FIND_CURRENT_HIGHLIGHT, this.createHighlight([active]))
    }
    return { all, current: active }
  }

  dispose(): void {
    this.revision++
    this.clear()
  }
}

interface RectOrigin {
  left: number
  top: number
}

function visibleRangeRects(range: Range): DOMRect[] {
  const fragments = Array.from(range.getClientRects()).filter((rect) => (
    Number.isFinite(rect.left)
    && Number.isFinite(rect.top)
    && Number.isFinite(rect.width)
    && Number.isFinite(rect.height)
    && (rect.width > 0 || rect.height > 0)
  ))
  if (fragments.length > 0) return fragments

  const bounds = range.getBoundingClientRect()
  return Number.isFinite(bounds.left)
    && Number.isFinite(bounds.top)
    && Number.isFinite(bounds.width)
    && Number.isFinite(bounds.height)
    && (bounds.width > 0 || bounds.height > 0)
    ? [bounds]
    : []
}

/** Convert viewport Range fragments to stable scroll-content overlay boxes. */
export function renderedHighlightRects(
  all: readonly Range[],
  current: Range | null,
  origin: RectOrigin,
  scrollLeft: number,
  scrollTop: number,
): RenderedHighlightRect[] {
  const output: RenderedHighlightRect[] = []

  function append(range: Range, isCurrent: boolean): void {
    for (const rect of visibleRangeRects(range)) {
      output.push({
        left: rect.left - origin.left + scrollLeft,
        top: rect.top - origin.top + scrollTop,
        width: Math.max(rect.width, 2),
        height: rect.height,
        current: isCurrent,
      })
    }
  }

  for (const range of all) append(range, false)
  if (current) append(current, true)
  return output
}
