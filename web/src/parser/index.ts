// Public surface of the tree-sitter parser module (S4).
// Browser bootstrap (loading wasm/.scm via Vite assets) lands in S7 with rendering.
export type { ParserLang, FunctionSpan, FileParse } from './types.ts';
export { FluidParser } from './parse.ts';
export type { LangAsset } from './parse.ts';
