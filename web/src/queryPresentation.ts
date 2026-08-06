import type { CodeEvidenceRef, QueryMap } from './ghostTypes'
import type { QueryEvidenceState } from './queryState'

export type QueryTurnSelection =
  | { kind: 'completed'; index: number }
  | { kind: 'active' }
  | null

export interface QueryTurnPresentationSnapshot {
  map: QueryMap
  evidence: QueryEvidenceState | null
  answerHtml: string
}

export interface QueryTurnPresentationState {
  selection: QueryTurnSelection
  snapshots: QueryTurnPresentationSnapshot[]
}

function completedTurnCount(value: number): number {
  return Number.isInteger(value) && value > 0 ? value : 0
}

/** Default focus target: an in-flight/error turn wins, otherwise the latest
 * completed pair. An empty trace has no selection. */
export function defaultQueryTurnSelection(
  completedCount: number,
  hasActiveTurn: boolean,
): QueryTurnSelection {
  if (hasActiveTurn) return activeQueryTurnSelection()
  const count = completedTurnCount(completedCount)
  return count > 0 ? { kind: 'completed', index: count - 1 } : null
}

export function activeQueryTurnSelection(): Exclude<QueryTurnSelection, null> {
  return { kind: 'active' }
}

export function completedQueryTurnSelection(
  index: number,
  completedCount: number,
): QueryTurnSelection {
  const count = completedTurnCount(completedCount)
  return Number.isInteger(index) && index >= 0 && index < count
    ? { kind: 'completed', index }
    : null
}

/** Preserve an explicit valid history choice; stale selections fall back to the
 * same active/latest rule used when focus mode opens. */
export function normalizeQueryTurnSelection(
  selection: QueryTurnSelection,
  completedCount: number,
  hasActiveTurn: boolean,
): QueryTurnSelection {
  if (selection?.kind === 'active' && hasActiveTurn) return selection
  if (selection?.kind === 'completed') {
    const completed = completedQueryTurnSelection(selection.index, completedCount)
    if (completed) return completed
  }
  return defaultQueryTurnSelection(completedCount, hasActiveTurn)
}

export function queryTurnSelectionKey(selection: QueryTurnSelection): string {
  if (!selection) return ''
  return selection.kind === 'active' ? 'active' : `completed:${selection.index}`
}

export function queryTurnSelectionFromKey(
  value: string,
  completedCount: number,
  hasActiveTurn: boolean,
): QueryTurnSelection {
  if (value === 'active') {
    return normalizeQueryTurnSelection({ kind: 'active' }, completedCount, hasActiveTurn)
  }
  const match = /^completed:(\d+)$/.exec(value)
  const completed = match
    ? completedQueryTurnSelection(Number(match[1]), completedCount)
    : null
  return completed ?? defaultQueryTurnSelection(completedCount, hasActiveTurn)
}

function snapshotEvidence(evidence: QueryEvidenceState | null): QueryEvidenceState | null {
  if (!evidence) return null
  return {
    ...evidence,
    sources: evidence.sources.map((source) => ({ ...source })),
  }
}

/** Append exactly one immutable presentation snapshot when a wire turn reaches
 * done. QueryTrace remains the domain/wire owner of the Q/A pair. */
export function appendQueryTurnPresentationSnapshot(
  snapshots: readonly QueryTurnPresentationSnapshot[],
  map: QueryMap,
  evidence: QueryEvidenceState | null,
): QueryTurnPresentationSnapshot[] {
  return [
    ...snapshots,
    {
      map,
      evidence: snapshotEvidence(evidence),
      answerHtml: '',
    },
  ]
}

/** Replace only the rendered presentation field while preserving map/evidence
 * alignment and leaving the previous snapshot array untouched. */
export function setQueryTurnAnswerHtml(
  snapshots: readonly QueryTurnPresentationSnapshot[],
  htmlByTurn: readonly string[],
): QueryTurnPresentationSnapshot[] {
  return snapshots.map((snapshot, index) => ({
    ...snapshot,
    answerHtml: htmlByTurn[index] ?? '',
  }))
}

/** E# identifiers are request-local, so historical lookup must start from the
 * selected turn snapshot rather than the latest QueryMap. */
export function queryTurnEvidenceById(
  snapshots: readonly QueryTurnPresentationSnapshot[],
  turnIndex: number,
  evidenceId: string,
): CodeEvidenceRef | undefined {
  return snapshots[turnIndex]?.map.evidence.find((reference) => reference.id === evidenceId)
}

export function resetQueryTurnPresentation(): QueryTurnPresentationState {
  return { selection: null, snapshots: [] }
}
