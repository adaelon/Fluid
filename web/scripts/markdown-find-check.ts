// S-FIND-MD1 deterministic checks. Run with:
//   node scripts/markdown-find-check.ts
// Node 24 strips TypeScript annotations. A tiny structural DOM double keeps this
// check dependency-free while exercising the production index/Range/highlight code.

import {
  collectInFileMatches,
  createInFileSearchQuery,
  type FindRange,
  type InFileFindQuery,
} from '../src/inFileFind.ts'
import {
  buildRenderedTextIndex,
  MARKDOWN_FIND_ALL_HIGHLIGHT,
  MARKDOWN_FIND_CURRENT_HIGHLIGHT,
  MarkdownFindHighlightLayer,
  renderedHighlightRects,
  renderedRangeForMatch,
} from '../src/markdownFind.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

class FakeStyle {
  display = 'inline'
  visibility = 'visible'
  opacity = '1'
  private values = new Map<string, string>()

  setProperty(name: string, value: string): void {
    this.values.set(name, value)
  }

  getPropertyValue(name: string): string {
    return this.values.get(name) ?? ''
  }
}

class FakeClassList {
  private values = new Set<string>()

  add(...names: string[]): void {
    for (const name of names) this.values.add(name)
  }

  contains(name: string): boolean {
    return this.values.has(name)
  }
}

class FakeNode {
  readonly nodeType: number
  childNodes: FakeNode[] = []
  parentElement: FakeElement | null = null
  ownerDocument: FakeDocument

  constructor(nodeType: number, ownerDocument: FakeDocument) {
    this.nodeType = nodeType
    this.ownerDocument = ownerDocument
  }
}

class FakeText extends FakeNode {
  readonly data: string

  constructor(data: string, ownerDocument: FakeDocument) {
    super(3, ownerDocument)
    this.data = data
  }
}

class FakeElement extends FakeNode {
  readonly tagName: string
  readonly classList = new FakeClassList()
  readonly computedStyle = new FakeStyle()
  readonly attributes = new Map<string, string>()
  hidden = false

  constructor(tagName: string, ownerDocument: FakeDocument) {
    super(1, ownerDocument)
    this.tagName = tagName
  }

  append(...children: FakeNode[]): this {
    for (const child of children) {
      child.parentElement = this
      child.ownerDocument = this.ownerDocument
      this.childNodes.push(child)
    }
    return this
  }

  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value)
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null
  }

  hasAttribute(name: string): boolean {
    return this.attributes.has(name)
  }
}

class FakeRange {
  startContainer: FakeText | null = null
  startOffset = 0
  endContainer: FakeText | null = null
  endOffset = 0
  detached = false

  setStart(node: FakeText, offset: number): void {
    this.startContainer = node
    this.startOffset = offset
  }

  setEnd(node: FakeText, offset: number): void {
    this.endContainer = node
    this.endOffset = offset
  }

  detach(): void {
    this.detached = true
  }
}

class FakeDocument {
  readonly defaultView = {
    getComputedStyle: (element: FakeElement) => element.computedStyle,
  }

  createRange(): FakeRange {
    return new FakeRange()
  }

  element(tag: string, ...children: FakeNode[]): FakeElement {
    return new FakeElement(tag.toUpperCase(), this).append(...children)
  }

  text(value: string): FakeText {
    return new FakeText(value, this)
  }
}

class FakeHighlightRegistry {
  readonly values = new Map<string, { ranges: FakeRange[] }>()

  set(name: string, highlight: { ranges: FakeRange[] }): void {
    this.values.set(name, highlight)
  }

  delete(name: string): boolean {
    return this.values.delete(name)
  }
}

const literal = (text: string): InFileFindQuery => ({
  text,
  mode: 'literal',
  caseSensitive: false,
})

function matches(source: ReturnType<typeof buildRenderedTextIndex>, text: string): FindRange[] {
  return collectInFileMatches(source.text, createInFileSearchQuery(literal(text)))
}

console.log('=== single node and UTF-16 DOM offsets ===')
{
  const doc = new FakeDocument()
  const content = doc.text('alpha beta')
  const root = doc.element('article', doc.element('p', content))
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  check('single text node is indexed verbatim', index.text.toString() === 'alpha beta')
  check('single node owns one stable segment', JSON.stringify(index.segments.map(({ from, to }) => ({ from, to }))) === JSON.stringify([
    { from: 0, to: 10 },
  ]))
  const range = renderedRangeForMatch(index, { from: 6, to: 10 }) as unknown as FakeRange
  check('single-node range maps exact start offset', range.startContainer === content && range.startOffset === 6)
  check('single-node range maps exact end offset', range.endContainer === content && range.endOffset === 10)
}

console.log('\n=== inline crossing and block boundaries ===')
{
  const doc = new FakeDocument()
  const alpha = doc.text('alpha ')
  const beta = doc.text('beta')
  const root = doc.element('article', doc.element('p', alpha, doc.element('em', beta)))
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  const cross = matches(index, 'alpha beta')
  check('adjacent inline nodes remain one searchable phrase', JSON.stringify(cross) === JSON.stringify([
    { from: 0, to: 10 },
  ]))
  const range = renderedRangeForMatch(index, cross[0]) as unknown as FakeRange
  check('cross-inline range starts in the first node', range.startContainer === alpha && range.startOffset === 0)
  check('cross-inline range ends in the second node', range.endContainer === beta && range.endOffset === 4)
}
{
  const doc = new FakeDocument()
  const first = doc.text('foo')
  const second = doc.text('bar')
  const root = doc.element(
    'article',
    doc.element('p', first),
    doc.text('\n  '),
    doc.element('p', second),
  )
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  check('block siblings receive one stable separator', index.text.toString() === 'foo\nbar')
  check('paragraph tail/head are not falsely concatenated', matches(index, 'foobar').length === 0)
  const range = renderedRangeForMatch(index, { from: 0, to: 7 }) as unknown as FakeRange
  check('cross-block range spans real endpoint nodes',
    range.startContainer === first && range.startOffset === 0
    && range.endContainer === second && range.endOffset === 3)
}

console.log('\n=== Chinese and fenced-code text ===')
{
  const doc = new FakeDocument()
  const chinese = doc.text('中文查找，再查找')
  const root = doc.element('article', doc.element('p', chinese))
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  check('Chinese matches retain UTF-16 offsets', JSON.stringify(matches(index, '查找')) === JSON.stringify([
    { from: 2, to: 4 },
    { from: 6, to: 8 },
  ]))
}
{
  const doc = new FakeDocument()
  const code = doc.text('const x = 1\n中文代码')
  const root = doc.element('article', doc.element('pre', doc.element('code', code)))
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  check('code-block newlines and text stay searchable', index.text.toString() === 'const x = 1\n中文代码')
  check('code-block Chinese result is collected', JSON.stringify(matches(index, '中文')) === JSON.stringify([
    { from: 12, to: 14 },
  ]))
}

console.log('\n=== hidden and duplicate auxiliary branches ===')
{
  const doc = new FakeDocument()
  const hidden = doc.element('span', doc.text('hidden'))
  hidden.hidden = true
  const ariaHidden = doc.element('span', doc.text('aria-hidden'))
  ariaHidden.setAttribute('aria-hidden', 'true')
  const displayNone = doc.element('span', doc.text('display-none'))
  displayNone.computedStyle.display = 'none'
  const mathml = doc.element('span', doc.text('duplicate-formula'))
  mathml.classList.add('katex-mathml')
  const visibleKatex = doc.element('span', doc.text('x+x'))
  visibleKatex.classList.add('katex-html')
  visibleKatex.setAttribute('aria-hidden', 'true')
  const root = doc.element(
    'article',
    doc.element(
      'p',
      doc.text('visible '),
      hidden,
      ariaHidden,
      displayNone,
      mathml,
      visibleKatex,
      doc.text(' end'),
    ),
  )
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  check('hidden and duplicate nodes are excluded', index.text.toString() === 'visible x+x end')
  check('the visually rendered KaTeX branch remains searchable', matches(index, 'x+x').length === 1)
}

console.log('\n=== revision-safe highlight cleanup ===')
{
  const doc = new FakeDocument()
  const root = doc.element('article', doc.element('p', doc.text('one one')))
  const index = buildRenderedTextIndex(root as unknown as HTMLElement)
  const found = matches(index, 'one')
  const registry = new FakeHighlightRegistry()
  const layer = new MarkdownFindHighlightLayer(
    registry as never,
    (ranges) => ({ ranges: ranges as unknown as FakeRange[] }) as never,
  )

  const firstRevision = layer.beginRevision()
  const firstPaint = layer.apply(firstRevision, index, found, 1)
  check('all and current highlight layers are both registered',
    registry.values.has(MARKDOWN_FIND_ALL_HIGHLIGHT)
    && registry.values.has(MARKDOWN_FIND_CURRENT_HIGHLIGHT))
  check('current layer contains only the selected result', firstPaint?.current === firstPaint?.all[0])

  const secondRevision = layer.beginRevision()
  check('beginning a revision removes every old Range registration', registry.values.size === 0)
  check('late work from a stale revision is rejected', layer.apply(firstRevision, index, found, 2) === null)
  check('stale work cannot restore deleted registrations', registry.values.size === 0)
  layer.apply(secondRevision, index, found, 2)
  const current = registry.values.get(MARKDOWN_FIND_CURRENT_HIGHLIGHT)?.ranges[0]
  check('the current layer moves to the requested result', current?.startOffset === 4 && current.endOffset === 7)
  layer.dispose()
  check('dispose clears all registrations', registry.values.size === 0)
}

console.log('\n=== Custom Highlight fallback overlay geometry ===')
{
  const range = {
    getClientRects: () => [{ left: 110, top: 220, width: 24, height: 12 }],
    getBoundingClientRect: () => ({ left: 110, top: 220, width: 24, height: 12 }),
  } as unknown as Range
  const boxes = renderedHighlightRects(
    [range],
    range,
    { left: 100, top: 200 },
    5,
    7,
  )
  check('fallback emits one all box and one current box', boxes.length === 2)
  check('fallback converts viewport coordinates to scroll content coordinates',
    boxes[0].left === 15 && boxes[0].top === 27
    && boxes[0].width === 24 && boxes[0].height === 12)
  check('fallback current box is layered last', !boxes[0].current && boxes[1].current)

  const fallbackLayer = new MarkdownFindHighlightLayer(null, () => ({}) as never)
  check('missing Custom Highlight registry selects the overlay path', !fallbackLayer.usesNativeHighlights())
}

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll markdown-find checks passed.')
