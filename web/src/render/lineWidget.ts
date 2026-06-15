// LineWidget — key-line ghost annotation that TRAILS its source line (需求 §7.3,
// ADR-0016). It is an *inline* widget placed at the END of the annotated line;
// CSS renders it as an inline-block a fixed gap to the right of the code, so each
// note starts just past its own line (not in a shared column) and wraps when
// long. Being inline-block it grows the line height, keeping code at the row top
// and the note fully visible without breaking the code column. Left bar carries
// the semantic color (#7ee787 normal / #f0883e branch / #ff7b72 exception).
// Shares the .fluid-ghost material with the capsule header (§7.4 continuity).

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
    const root = document.createElement('span')
    root.className = 'fluid-ghost fluid-line-anno'
    root.style.setProperty('--c', this.ln.color)
    root.textContent = this.ln.text
    return root
  }
}
