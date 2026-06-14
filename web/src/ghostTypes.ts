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
