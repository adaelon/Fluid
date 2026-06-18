// GhostStore — in-memory ghost annotations for the *currently open* file
// (ADR-0002, 技术方案 §2). The single source of truth the CM6 decoration field
// projects from. Folding hides, never deletes: a folded function's capsule and
// lines stay in memory so unfolding is pure re-render, zero recompute (需求 §7.5).
// Plain class (no Vue reactivity): held via the editor's imperative side and
// mutated directly, then the view is asked to refresh (ADR-0014).

import type { DeclSpan, FunctionSpan } from './parser/types.ts'
import type { Capsule, LineAnnotation } from './ghostTypes'

/** Generation status of one function (S7.5): request in flight, finished, or failed. */
export type GhostStatus = 'pending' | 'settled' | 'error'

export class GhostStore {
  /** Function roster for the open file (positions the capsules/lines). */
  roster: FunctionSpan[] = []
  /** Top-level declarations (TS, S-TS-3): manual-explain entries, no auto-gen. */
  decls: DeclSpan[] = []
  private capsules = new Map<string, Capsule>()
  private lineMap = new Map<string, LineAnnotation[]>()
  private keyLineMap = new Map<string, number[]>()
  private folded = new Set<string>()
  /** Per-function generation status (S7.5): drives the "生成中" placeholder. */
  private status = new Map<string, GhostStatus>()
  /** Last error message per function (S7.6): shown on the "生成失败" chip. */
  private errors = new Map<string, string>()
  /** In-flight manual line fills (S9): `${fnId}:${lineNumber}` → "解释中…" hotspot. */
  private explaining = new Set<string>()

  /** Drop everything — called on file close / switch (releases memory, §7 VACUUM). */
  reset(): void {
    this.roster = []
    this.decls = []
    this.capsules.clear()
    this.lineMap.clear()
    this.keyLineMap.clear()
    this.folded.clear()
    this.status.clear()
    this.errors.clear()
    this.explaining.clear()
  }

  /** Establish the functions to render and their key lines (from tree-sitter). */
  setRoster(roster: FunctionSpan[], keyLines: Map<string, number[]>): void {
    this.roster = roster
    this.keyLineMap = keyLines
  }

  /** Establish the top-level declarations offered for manual explain (S-TS-3). */
  setDecls(decls: DeclSpan[]): void {
    this.decls = decls
  }

  keyLinesOf(fnId: string): number[] {
    return this.keyLineMap.get(fnId) ?? []
  }

  putCapsule(c: Capsule): void {
    this.capsules.set(c.fnId, c)
  }

  capsule(fnId: string): Capsule | undefined {
    return this.capsules.get(fnId)
  }

  /** Add or replace a line annotation (re-arrival of the same line replaces). */
  putLine(l: LineAnnotation): void {
    const arr = this.lineMap.get(l.fnId) ?? []
    const i = arr.findIndex((x) => x.lineNumber === l.lineNumber)
    if (i >= 0) arr[i] = l
    else arr.push(l)
    arr.sort((p, q) => p.lineNumber - q.lineNumber)
    this.lineMap.set(l.fnId, arr)
  }

  lines(fnId: string): LineAnnotation[] {
    return this.lineMap.get(fnId) ?? []
  }

  isFolded(fnId: string): boolean {
    return this.folded.has(fnId)
  }

  toggleFold(fnId: string): void {
    if (this.folded.has(fnId)) this.folded.delete(fnId)
    else this.folded.add(fnId)
  }

  // — generation status (S7.5) —

  markPending(fnId: string): void {
    this.status.set(fnId, 'pending')
    this.errors.delete(fnId)
  }

  /** Mark a function's generation finished (`ok`) or failed (with a message). */
  settle(fnId: string, ok: boolean, message = ''): void {
    this.status.set(fnId, ok ? 'settled' : 'error')
    if (ok) this.errors.delete(fnId)
    else this.errors.set(fnId, message)
  }

  statusOf(fnId: string): GhostStatus | undefined {
    return this.status.get(fnId)
  }

  errorOf(fnId: string): string {
    return this.errors.get(fnId) ?? ''
  }

  // — manual line fill in-flight (S9) —

  private explainKey(fnId: string, lineNumber: number): string {
    return `${fnId}:${lineNumber}`
  }

  markExplaining(fnId: string, lineNumber: number): void {
    this.explaining.add(this.explainKey(fnId, lineNumber))
  }

  clearExplaining(fnId: string, lineNumber: number): void {
    this.explaining.delete(this.explainKey(fnId, lineNumber))
  }

  isExplaining(fnId: string, lineNumber: number): boolean {
    return this.explaining.has(this.explainKey(fnId, lineNumber))
  }
}
