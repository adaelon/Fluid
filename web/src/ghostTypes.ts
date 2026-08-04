// Ghost-annotation wire types — mirror the Rust domain types (cache_store.rs /
// 技术方案 §3) and the WS frame protocol (routes.rs GenFrame, §4). Hand-written
// for now; ts-rs generation (ADR-0013) is not yet wired.

/** Function-granularity semantic capsule (技术方案 §3). */
export interface Capsule {
  fnId: string
  signature: string
  summary: string
  complexity: string
  io: string
}

/** Line-level ghost annotation on a key line (技术方案 §3). */
export interface LineAnnotation {
  fnId: string
  lineNumber: number
  text: string
  color: string
}

/** One inbound frame from `WS /api/generate` (S7a, §4). `reqId` echoes the
 *  request (= the function id); terminal frames are `done` | `error`. */
export type GenFrame =
  | { kind: 'cache-hit'; reqId: string }
  | { kind: 'capsule'; reqId: string; capsule: Capsule }
  | { kind: 'line'; reqId: string; line: LineAnnotation }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }

/** One inbound frame from `WS /api/query` (S10a, routes.rs QueryFrame). The answer
 *  is free-form markdown streamed as `delta` chunks; terminal frames are
 *  `done` | `error`. `reqId` echoes the request. */
export type QueryFrame =
  | { kind: 'delta'; reqId: string; text: string }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }

/** Evidence metadata shared by selection explanations and the later query-Web
 * slice. `web-uncited` is a successful supplier search with no returned URLs. */
export type EvidenceStatus = 'project-source' | 'web-cited' | 'web-uncited' | 'unverified'

export interface SourceLink {
  title: string
  url: string
}

export type SelectionKind = '模块' | '类型' | '函数' | '方法' | '变量' | '表达式' | '未知'

export interface SelectionExplanation {
  selectedText: string
  kind: SelectionKind
  meaning: string
  roleHere: string
  origin?: string
  evidenceStatus: EvidenceStatus
  sources?: SourceLink[]
  warning?: string
}

export type SelectionPhase =
  | 'resolving-project'
  | 'planning-web'
  | 'searching-web'
  | 'answering'
  | 'fallback'

/** One inbound frame from `WS /api/explain-selection` (S-SEL-1). */
export type SelectionFrame =
  | { kind: 'cache-hit'; reqId: string }
  | { kind: 'status'; reqId: string; phase: SelectionPhase; message: string }
  | { kind: 'result'; reqId: string; explanation: SelectionExplanation }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }
