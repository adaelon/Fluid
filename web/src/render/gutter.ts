// gutter.ts — S9 frontend: discovery affordances for manual single-line fill.
//
// Two pieces, both deferring to the GhostStore as the source of truth:
//   1. fnGutter — a hollow 2px dot in the gutter at each function's start line
//      (需求 §7.2). Pure discovery hint; recomputed on the same `refreshGhosts`
//      signal the decoration field uses, so it appears once the roster is set.
//   2. ExplainHotspotWidget + explainClickHandler — a "解释这一行" affordance that
//      hover-reveals at the end of eligible NON-key lines (inside a generated
//      function, not a key line, not yet annotated, not blank). Clicking it asks
//      the backend (POST /api/explain-line) to fill that one line (CONTEXT 手动补行).
//
// Per ADR-0016 (line notes are visible by default) the old key-line *solid pulse
// dot* is dropped — the trailing inline note already marks key lines, so a pulsing
// gutter dot would be redundant. Only the function-top hollow dot remains.

import { EditorView, GutterMarker, WidgetType, gutter } from '@codemirror/view'
import type { GhostStore } from '../ghostStore'
import { refreshGhosts } from './ghostField'

class FnDotMarker extends GutterMarker {
  toDOM(): Node {
    const el = document.createElement('span')
    el.className = 'fluid-gutter-fn-dot'
    return el
  }
}
const fnDot = new FnDotMarker()

/** A hollow gutter dot on every function's start line (§7.2 discovery hint). */
export function fnGutter(store: GhostStore) {
  return gutter({
    class: 'fluid-fn-gutter',
    lineMarker(view, line) {
      const n = view.state.doc.lineAt(line.from).number
      return store.roster.some((fn) => fn.lineRange[0] === n) ? fnDot : null
    },
    // Recompute markers when the store changes (roster set / generation refresh).
    lineMarkerChange(update) {
      return update.transactions.some((tr) => tr.effects.some((e) => e.is(refreshGhosts)))
    },
  })
}

/** Hover-revealed "解释这一行" affordance trailing an eligible non-key line. While
 *  the request is in flight it switches to a non-clickable "解释中…" pulse. */
export class ExplainHotspotWidget extends WidgetType {
  constructor(
    readonly fnId: string,
    readonly lineNumber: number,
    readonly loading: boolean,
  ) {
    super()
  }

  eq(other: ExplainHotspotWidget): boolean {
    return (
      other.fnId === this.fnId &&
      other.lineNumber === this.lineNumber &&
      other.loading === this.loading
    )
  }

  toDOM(): HTMLElement {
    const el = document.createElement('span')
    if (this.loading) {
      el.className = 'fluid-explain-hotspot loading'
      el.textContent = '解释中…'
    } else {
      el.className = 'fluid-explain-hotspot'
      el.textContent = '解释这一行'
      el.setAttribute('data-explain-fn', this.fnId)
      el.setAttribute('data-explain-line', String(this.lineNumber))
    }
    return el
  }

  // Let the click reach the dom event handler (mirrors the capsule/retry widgets).
  ignoreEvent(): boolean {
    return false
  }
}

/** Click handler: trigger manual line fill when a "解释这一行" hotspot is clicked. */
export function explainClickHandler(onExplain: (fnId: string, lineNumber: number) => void) {
  return EditorView.domEventHandlers({
    mousedown(e) {
      const target = e.target as HTMLElement | null
      const el = target?.closest('[data-explain-fn]')
      if (!el) return false
      const fnId = el.getAttribute('data-explain-fn')
      const ln = el.getAttribute('data-explain-line')
      if (!fnId || !ln) return false
      onExplain(fnId, Number(ln))
      e.preventDefault()
      return true
    },
  })
}
