import type { CodeEvidenceRef, QueryMap } from './ghostTypes'
import type { QueryThreadFreshness, QueryThreadStaleReason } from './api'

/** Historical code coordinates are only valid while the server says the
 * thread still matches current project bytes. No selected thread means the
 * active live query path, whose evidence remains interactive. */
export function queryCodeEvidenceNavigationEnabled(
  freshness: QueryThreadFreshness | undefined,
): boolean {
  return freshness !== 'stale'
}

/** Keep the two backend stale classifications visible and action-oriented. */
export function queryStaleReasonMessage(
  reason: QueryThreadStaleReason | undefined,
): string {
  return reason === 'source-missing'
    ? '范围文件缺失，旧源码证据当前不可回切'
    : '源码已变更，旧源码证据当前不可回切'
}

/** Collect dangling E# references from backend map structure. The UI keeps this
 * guard even though the Rust producer validates maps, so malformed/older peers
 * fail visibly instead of rendering a dead evidence control. */
export function queryMapUnknownEvidenceIds(map: QueryMap): string[] {
  const known = new Set(map.evidence.map((reference) => reference.id))
  const unknown = new Set<string>()
  const inspect = (ids: string[]) => {
    for (const id of ids) if (!known.has(id)) unknown.add(id)
  }
  for (const step of map.direction) inspect(step.evidenceIds)
  for (const step of map.walkthrough.steps) inspect(step.evidenceIds)
  return Array.from(unknown)
}

export function queryEvidenceById(
  evidence: CodeEvidenceRef[],
  id: string,
): CodeEvidenceRef | undefined {
  return evidence.find((reference) => reference.id === id)
}

export interface QueryEvidenceCitations {
  knownIds: string[]
  unknownIds: string[]
}

/** Extract model-written [E#] citations while ignoring fenced and inline code.
 * Markdown rendering performs the same known-ID check before making a citation
 * clickable; this pure scan drives trace metadata and the visible unknown-ID
 * warning without trusting arbitrary model links. */
export function queryAnswerEvidenceCitations(
  answer: string,
  evidence: CodeEvidenceRef[],
): QueryEvidenceCitations {
  const known = new Set(evidence.map((reference) => reference.id))
  const knownIds = new Set<string>()
  const unknownIds = new Set<string>()
  const prose = answer
    .replace(/(^|\n)(```|~~~)[^\n]*\n[\s\S]*?\n\2(?=\n|$)/g, '$1')
    .replace(/`+[^`\n]*`+/g, '')
  for (const match of prose.matchAll(/\[(E[1-9]\d*)\]/g)) {
    const id = match[1]
    if (known.has(id)) knownIds.add(id)
    else unknownIds.add(id)
  }
  return {
    knownIds: Array.from(knownIds),
    unknownIds: Array.from(unknownIds),
  }
}
