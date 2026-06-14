// ghostField — projects a GhostStore into CM6 block-widget decorations.
//
// The store is the source of truth; this field is a pure view of it. A
// `refreshGhosts` effect (dispatched whenever the store mutates) rebuilds the
// decoration set from current store state. Because the doc is read-only and the
// data lives in the store, fold/unfold is just a rebuild that omits/includes the
// hidden widgets — zero recompute, no refetch (需求 §7.5).

import { StateEffect, StateField, type EditorState, type Range } from '@codemirror/state'
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view'
import type { GhostStore } from '../ghostStore'
import { CapsuleWidget } from './capsuleWidget'
import { LineWidget } from './lineWidget'
import { PlaceholderWidget } from './placeholderWidget'

/** Dispatched after the store changes to ask the field to re-project. */
export const refreshGhosts = StateEffect.define<void>()

function build(store: GhostStore, state: EditorState): DecorationSet {
  const docLines = state.doc.lines
  const ranges: Range<Decoration>[] = []

  for (const fn of store.roster) {
    const start = fn.lineRange[0]
    if (start < 1 || start > docLines) continue
    const folded = store.isFolded(fn.id)

    const at = state.doc.line(start).from
    const cap = store.capsule(fn.id)
    if (cap) {
      ranges.push(
        Decoration.widget({ widget: new CapsuleWidget(cap, folded), block: true, side: -1 }).range(at),
      )
    } else {
      // No capsule yet (S7.5): show a "生成中" skeleton, or a "生成失败" chip.
      const st = store.statusOf(fn.id)
      if (st === 'pending' || st === 'error') {
        const widget =
          st === 'error'
            ? new PlaceholderWidget(fn.id, 'error', store.errorOf(fn.id))
            : new PlaceholderWidget(fn.id, 'pending')
        ranges.push(Decoration.widget({ widget, block: true, side: -1 }).range(at))
      }
    }

    if (!folded) {
      for (const ln of store.lines(fn.id)) {
        if (ln.lineNumber < 1 || ln.lineNumber > docLines) continue
        const at = state.doc.line(ln.lineNumber).from
        ranges.push(
          Decoration.widget({ widget: new LineWidget(ln), block: true, side: -1 }).range(at),
        )
      }
    }
  }

  // Decoration.set sorts by position; block widgets at distinct lines don't collide.
  return Decoration.set(ranges, true)
}

/** Build the decoration field bound to a specific store instance. */
export function ghostField(store: GhostStore): StateField<DecorationSet> {
  return StateField.define<DecorationSet>({
    create: (state) => build(store, state),
    update(deco, tr) {
      if (tr.effects.some((e) => e.is(refreshGhosts))) return build(store, tr.state)
      return tr.docChanged ? deco.map(tr.changes) : deco
    },
    provide: (f) => EditorView.decorations.from(f),
  })
}

/** Click handler: toggle fold when a capsule (its `[data-fold]` root) is clicked. */
export function foldClickHandler(store: GhostStore) {
  return EditorView.domEventHandlers({
    mousedown(e, view) {
      const target = e.target as HTMLElement | null
      const el = target?.closest('[data-fold]')
      if (!el) return false
      const id = el.getAttribute('data-fold')
      if (!id) return false
      store.toggleFold(id)
      view.dispatch({ effects: refreshGhosts.of() })
      e.preventDefault()
      return true
    },
  })
}

/** Click handler: re-run generation for a failed function (its 重试 button). */
export function retryClickHandler(onRetry: (fnId: string) => void) {
  return EditorView.domEventHandlers({
    mousedown(e) {
      const target = e.target as HTMLElement | null
      const el = target?.closest('[data-retry]')
      if (!el) return false
      const id = el.getAttribute('data-retry')
      if (!id) return false
      onRetry(id)
      e.preventDefault()
      return true
    },
  })
}
