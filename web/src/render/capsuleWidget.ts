// CapsuleWidget — CM6 block widget rendering a function capsule above its
// definition line (需求 §7.2). Folded: collapses to a 4px color edge memory
// anchor (§7.5). Same glass pipeline as LineWidget per §7.4 (the .fluid-ghost
// base class); reveal animation is CSS, triggered when fresh DOM is created.

import { WidgetType } from '@codemirror/view'
import type { Capsule } from '../ghostTypes'

/** Neutral capsule accent (capsules carry no semantic color; lines do). */
const CAPSULE_ACCENT = '#58a6ff'

export class CapsuleWidget extends WidgetType {
  constructor(
    readonly cap: Capsule,
    readonly folded: boolean,
  ) {
    super()
  }

  // Reuse DOM (no re-animation) unless content or fold state changed.
  eq(other: CapsuleWidget): boolean {
    return (
      other.cap.fnId === this.cap.fnId &&
      other.folded === this.folded &&
      other.cap.summary === this.cap.summary &&
      other.cap.complexity === this.cap.complexity &&
      other.cap.io === this.cap.io
    )
  }

  toDOM(): HTMLElement {
    const root = document.createElement('div')
    root.className = 'fluid-ghost fluid-capsule' + (this.folded ? ' folded' : '')
    root.setAttribute('data-fold', this.cap.fnId)
    root.title = this.folded ? '展开函数胶囊' : '折叠函数胶囊'
    root.style.setProperty('--c', CAPSULE_ACCENT)

    if (this.folded) return root

    // Single-line header: summary … complexity · io (§7.2, ADR-0016).
    const sum = document.createElement('span')
    sum.className = 'fluid-cap-sum'
    sum.textContent = this.cap.summary || ''

    const meta = document.createElement('span')
    meta.className = 'fluid-cap-meta'
    meta.textContent = [this.cap.complexity, this.cap.io].filter(Boolean).join(' · ')

    root.append(sum, meta)
    return root
  }

  // Let the click reach the editor's fold handler (domEventHandlers).
  ignoreEvent(): boolean {
    return false
  }
}
