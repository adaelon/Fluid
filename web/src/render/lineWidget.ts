// LineWidget — CM6 block widget rendering a key-line ghost annotation above the
// annotated source line (需求 §7.3). Left bar uses the annotation's semantic
// color (#7ee787 normal / #f0883e branch / #ff7b72 exception). Shares the
// .fluid-ghost glass pipeline with CapsuleWidget (§7.4 material continuity).

import { WidgetType } from '@codemirror/view'
import type { LineAnnotation } from '../ghostTypes'

export class LineWidget extends WidgetType {
  constructor(readonly ln: LineAnnotation) {
    super()
  }

  eq(other: LineWidget): boolean {
    return (
      other.ln.fnId === this.ln.fnId &&
      other.ln.lineNumber === this.ln.lineNumber &&
      other.ln.text === this.ln.text &&
      other.ln.color === this.ln.color
    )
  }

  toDOM(): HTMLElement {
    const root = document.createElement('div')
    root.className = 'fluid-ghost fluid-line'

    const bar = document.createElement('span')
    bar.className = 'fluid-bar'
    bar.style.setProperty('--c', this.ln.color)

    const body = document.createElement('div')
    body.className = 'fluid-body'

    const num = document.createElement('span')
    num.className = 'fluid-linenum'
    num.textContent = 'L' + this.ln.lineNumber

    const text = document.createElement('span')
    text.className = 'fluid-linetext'
    text.textContent = this.ln.text

    body.append(num, text)
    root.append(bar, body)
    return root
  }
}
