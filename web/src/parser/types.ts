/** Languages Fluid's parser supports (ADR-0005: py/rs phase 1; ts added S-TS-2). */
export type ParserLang = 'py' | 'rs' | 'ts';

/** One function in a file's roster: name + 1-indexed inclusive line range. */
export interface FunctionSpan {
  /** Unique within a file: `${name}#${startLine}` (disambiguates same-named methods). */
  id: string;
  name: string;
  /** [startLine, endLine], 1-indexed, inclusive. */
  lineRange: [number, number];
}

/** tree-sitter parse product for one file (技术方案 §3). */
export interface FileParse {
  roster: FunctionSpan[];
  /** fnId -> sorted, deduped key-line numbers (1-indexed). */
  keyLines: Map<string, number[]>;
}
