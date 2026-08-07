import { getSearchQuery, SearchQuery } from '@codemirror/search'
import type { EditorState, Text as CmText } from '@codemirror/state'

export type InFileFindMode = 'literal' | 'regexp'
export type InFileFindDirection = 'next' | 'previous'

export interface InFileFindQuery {
  text: string
  mode: InFileFindMode
  caseSensitive: boolean
}

export interface InFileFindSnapshot {
  /** 1-based match number; zero when there is no current match. */
  current: number
  total: number
  error: 'invalid-regexp' | null
}

export interface InFileFindSurfaceHandle {
  moveFind(direction: InFileFindDirection): void
  focusContent(): void
}

/** UTF-16 document offsets shared by CodeMirror and browser DOM ranges. */
export interface FindRange {
  from: number
  to: number
}

/** Keep literal and regexp searches on one CodeMirror-owned matching contract. */
export function createInFileSearchQuery(query: InFileFindQuery): SearchQuery {
  return new SearchQuery({
    search: query.text,
    caseSensitive: query.caseSensitive,
    literal: query.mode === 'literal',
    regexp: query.mode === 'regexp',
    replace: '',
    wholeWord: false,
  })
}

/** Collect stable, document-ordered ranges without interpreting the query again. */
export function collectInFileMatches(
  text: CmText | EditorState,
  query: SearchQuery,
): FindRange[] {
  if (!query.valid) return []

  const matches: FindRange[] = []
  const cursor = query.getCursor(text)
  for (let step = cursor.next(); !step.done; step = cursor.next()) {
    matches.push({ from: step.value.from, to: step.value.to })
  }
  return matches
}

/** Resolve an active range to the 1-based counter shown by the find surface. */
export function currentInFileMatch(
  matches: readonly FindRange[],
  activeRange: FindRange | null,
): number {
  if (!activeRange) return 0
  const index = matches.findIndex(
    (match) => match.from === activeRange.from && match.to === activeRange.to,
  )
  return index + 1
}

/** Read the controlled query and active selection from CodeMirror search state. */
export function snapshotInFileFindState(state: EditorState): InFileFindSnapshot {
  const query = getSearchQuery(state)
  const matches = collectInFileMatches(state, query)
  const selection = state.selection.main
  return {
    current: currentInFileMatch(matches, { from: selection.from, to: selection.to }),
    total: matches.length,
    error: query.regexp && query.search.length > 0 && !query.valid
      ? 'invalid-regexp'
      : null,
  }
}

/** Move a 1-based current counter, wrapping at both ends. */
export function moveInFileFindCurrent(
  current: number,
  total: number,
  direction: InFileFindDirection,
): number {
  if (!Number.isInteger(total) || total <= 0) return 0
  if (!Number.isInteger(current) || current < 1 || current > total) {
    return direction === 'next' ? 1 : total
  }
  if (direction === 'next') return current === total ? 1 : current + 1
  return current === 1 ? total : current - 1
}
