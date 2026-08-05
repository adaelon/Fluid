// CapsuleWidget — CM6 block widget rendering a function capsule above its
// definition line (需求 §7.2). Folded: collapses to a 4px color edge memory
// anchor (§7.5). Same glass pipeline as LineWidget per §7.4 (the .fluid-ghost
// base class); reveal animation is CSS, triggered when fresh DOM is created.

import { WidgetType } from '@codemirror/view'
import { capsulePresentation } from '../capsulePresentation'
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
    const current = capsulePresentation(this.cap)
    const next = capsulePresentation(other.cap)
    return (
      other.cap.fnId === this.cap.fnId &&
      other.folded === this.folded &&
      other.cap.orientationId === this.cap.orientationId &&
      other.cap.summary === this.cap.summary &&
      other.cap.complexity === this.cap.complexity &&
      other.cap.io === this.cap.io &&
      next.lane === current.lane &&
      next.stage === current.stage &&
      next.direction === current.direction &&
      next.why === current.why &&
      next.evidence === current.evidence
    )
  }

  toDOM(): HTMLElement {
    const root = document.createElement('div')
    root.className = 'fluid-ghost fluid-capsule' + (this.folded ? ' folded' : '')
    root.setAttribute('data-fold', this.cap.fnId)
    root.setAttribute('data-orientation-id', this.cap.orientationId)
    root.setAttribute('data-function-lane', this.cap.role.lane)
    root.title = this.folded ? '展开函数胶囊' : '折叠函数胶囊'
    root.style.setProperty('--c', CAPSULE_ACCENT)

    if (this.folded) return root

    const presentation = capsulePresentation(this.cap)

    // Summary and the original local complexity/io projection remain visible.
    const sum = document.createElement('span')
    sum.className = 'fluid-cap-sum'
    sum.textContent = this.cap.summary || ''

    const meta = document.createElement('span')
    meta.className = 'fluid-cap-meta'
    meta.textContent = [this.cap.complexity, this.cap.io].filter(Boolean).join(' · ')

    const coordinates = document.createElement('div')
    coordinates.className = 'fluid-cap-coordinates'

    const lane = document.createElement('span')
    lane.className = `fluid-cap-lane ${this.cap.role.lane}`
    lane.textContent = presentation.lane

    const stage = document.createElement('span')
    stage.className = 'fluid-cap-stage'
    stage.textContent = presentation.stage

    const direction = document.createElement('code')
    direction.className = 'fluid-cap-direction'
    direction.textContent = presentation.direction
    coordinates.append(lane, stage, direction)

    const details = document.createElement('details')
    details.className = 'fluid-cap-details'
    details.setAttribute('data-capsule-details', '')
    // The capsule root uses mousedown to fold. Keep native <details> interaction
    // local so expanding evidence never folds the whole capsule.
    details.addEventListener('mousedown', (event) => event.stopPropagation())

    const detailsSummary = document.createElement('summary')
    detailsSummary.textContent = '角色依据'
    const why = document.createElement('span')
    why.className = 'fluid-cap-why'
    why.textContent = presentation.why
    const evidence = document.createElement('span')
    evidence.className = 'fluid-cap-evidence'
    evidence.textContent = `源码证据：${presentation.evidence}`
    details.append(detailsSummary, why, evidence)

    root.append(sum, meta, coordinates, details)
    return root
  }

  // Let the click reach the editor's fold handler (domEventHandlers).
  ignoreEvent(): boolean {
    return false
  }
}
