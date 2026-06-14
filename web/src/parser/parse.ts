// FluidParser — tree-sitter (WASM) function roster + key-line extraction.
//
// S4 scope (ADR-0005/0009): deterministic, zero-token parsing of Python & Rust.
// Environment-agnostic: callers supply grammar bytes + key-line query source, so
// the same core runs in the browser (S7) and in the Node validation script.

import Parser from 'web-tree-sitter';
import type { FileParse, FunctionSpan, ParserLang } from './types.ts';

/** Structural function enumeration per language (the roster; not a heuristic). */
const ROSTER_QUERY: Record<ParserLang, string> = {
  py: '(function_definition name: (identifier) @name) @fn',
  rs: '(function_item name: (identifier) @name) @fn',
};

/** What a caller provides to enable one language. */
export interface LangAsset {
  lang: ParserLang;
  /** Raw bytes of the tree-sitter grammar wasm. */
  grammarWasm: Uint8Array;
  /** Contents of the matching keyline-queries/*.scm file. */
  keyLineQuery: string;
}

interface Compiled {
  language: Parser.Language;
  rosterQuery: Parser.Query;
  keyLineQuery: Parser.Query;
}

export class FluidParser {
  private readonly parser: Parser;
  private readonly langs: Map<ParserLang, Compiled>;

  private constructor(parser: Parser, langs: Map<ParserLang, Compiled>) {
    this.parser = parser;
    this.langs = langs;
  }

  /** Initialize the WASM runtime and compile queries for the given languages. */
  static async create(assets: LangAsset[], initOptions?: object): Promise<FluidParser> {
    await Parser.init(initOptions);
    const parser = new Parser();
    const langs = new Map<ParserLang, Compiled>();
    for (const a of assets) {
      const language = await Parser.Language.load(a.grammarWasm);
      langs.set(a.lang, {
        language,
        rosterQuery: language.query(ROSTER_QUERY[a.lang]),
        keyLineQuery: language.query(a.keyLineQuery),
      });
    }
    return new FluidParser(parser, langs);
  }

  supports(lang: string): lang is ParserLang {
    return this.langs.has(lang as ParserLang);
  }

  /** Parse one file into its function roster + key-line map. */
  parse(lang: ParserLang, source: string): FileParse {
    const compiled = this.langs.get(lang);
    if (!compiled) throw new Error(`parser: unsupported lang "${lang}"`);
    this.parser.setLanguage(compiled.language);
    const tree = this.parser.parse(source);
    const roster = extractRoster(compiled.rosterQuery, tree);
    const keyLines = extractKeyLines(compiled.keyLineQuery, tree, roster);
    return { roster, keyLines };
  }
}

function extractRoster(query: Parser.Query, tree: Parser.Tree): FunctionSpan[] {
  const out: FunctionSpan[] = [];
  for (const m of query.matches(tree.rootNode)) {
    const fn = m.captures.find((c) => c.name === 'fn')?.node;
    const name = m.captures.find((c) => c.name === 'name')?.node;
    if (!fn || !name) continue;
    const start = fn.startPosition.row + 1;
    const end = fn.endPosition.row + 1;
    out.push({ id: `${name.text}#${start}`, name: name.text, lineRange: [start, end] });
  }
  return out;
}

function extractKeyLines(
  query: Parser.Query,
  tree: Parser.Tree,
  roster: FunctionSpan[],
): Map<string, number[]> {
  const sets = new Map<string, Set<number>>();
  for (const cap of query.captures(tree.rootNode)) {
    const line = cap.node.startPosition.row + 1;
    const host = innermostHost(roster, line);
    if (!host) continue; // module/class-level line: not attached to any capsule
    let set = sets.get(host.id);
    if (!set) sets.set(host.id, (set = new Set<number>()));
    set.add(line);
  }
  const out = new Map<string, number[]>();
  for (const [id, set] of sets) out.set(id, [...set].sort((a, b) => a - b));
  return out;
}

/** The function span most tightly enclosing `line` (innermost wins for nesting). */
function innermostHost(roster: FunctionSpan[], line: number): FunctionSpan | undefined {
  let best: FunctionSpan | undefined;
  for (const r of roster) {
    if (r.lineRange[0] <= line && line <= r.lineRange[1]) {
      if (!best || r.lineRange[0] > best.lineRange[0]) best = r;
    }
  }
  return best;
}
