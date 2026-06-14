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
      other.cap.signature === this.cap.signature &&
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

    const bar = document.createElement('span')
    bar.className = 'fluid-bar'
    bar.style.setProperty('--c', CAPSULE_ACCENT)
    root.appendChild(bar)

    if (this.folded) return root

    const body = document.createElement('div')
    body.className = 'fluid-body'

    const sig = document.createElement('div')
    sig.className = 'fluid-sig'
    sig.textContent = this.cap.signature || this.cap.fnId

    const summary = document.createElement('div')
    summary.className = 'fluid-summary'
    summary.textContent = this.cap.summary

    const meta = document.createElement('div')
    meta.className = 'fluid-meta'
    meta.textContent = [this.cap.complexity, this.cap.io].filter(Boolean).join(' · ')

    body.append(sig, summary, meta)
    root.appendChild(body)
    return root
  }

  // Let the click reach the editor's fold handler (domEventHandlers).
  ignoreEvent(): boolean {
    return false
  }
}
