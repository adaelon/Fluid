// FluidParser — tree-sitter (WASM) function roster + key-line extraction.
//
// S4 scope (ADR-0005/0009): deterministic, zero-token parsing of Python & Rust.
// Environment-agnostic: callers supply grammar bytes + key-line query source, so
// the same core runs in the browser (S7) and in the Node validation script.

import Parser from 'web-tree-sitter';
import type { DeclSpan, FileParse, FunctionSpan, ParserLang } from './types.ts';

/** Structural function enumeration per language (the roster; not a heuristic). */
const ROSTER_QUERY: Record<ParserLang, string> = {
  py: '(function_definition name: (identifier) @name) @fn',
  rs: '(function_item name: (identifier) @name) @fn',
  // TS has several "function" shapes; @fn must cover a body so its key lines have a
  // host (innermostHost). Named function decls, class methods, and the very common
  // `const f = () => {}` / `const f = function(){}` / class field arrow `f = () => {}`.
  ts: [
    '(function_declaration name: (identifier) @name) @fn',
    '(generator_function_declaration name: (identifier) @name) @fn',
    '(method_definition name: (property_identifier) @name) @fn',
    '(variable_declarator name: (identifier) @name value: (arrow_function)) @fn',
    '(variable_declarator name: (identifier) @name value: (function_expression)) @fn',
    '(public_field_definition name: (property_identifier) @name value: (arrow_function)) @fn',
  ].join('\n'),
};

/** Top-level declarations eligible for manual "explain this declaration" (S-TS-3).
 *  TS only. Matches MODULE-LEVEL const/let/type/interface/enum (program direct
 *  children + export-wrapped); function-bodies' decls are excluded (those are
 *  already covered by 手动补行 on non-key lines). `@decl` is the declaration node
 *  (sets the line range), `@name` its identifier. Function-valued consts also match
 *  here but are filtered out downstream (they're roster functions, deduped by start
 *  line). */
const DECL_QUERY: Partial<Record<ParserLang, string>> = {
  ts: [
    '(program (lexical_declaration (variable_declarator name: (identifier) @name)) @decl)',
    '(program (type_alias_declaration name: (type_identifier) @name) @decl)',
    '(program (interface_declaration name: (type_identifier) @name) @decl)',
    '(program (enum_declaration name: (identifier) @name) @decl)',
    '(program (export_statement (lexical_declaration (variable_declarator name: (identifier) @name)) @decl))',
    '(program (export_statement (type_alias_declaration name: (type_identifier) @name) @decl))',
    '(program (export_statement (interface_declaration name: (type_identifier) @name) @decl))',
    '(program (export_statement (enum_declaration name: (identifier) @name) @decl))',
  ].join('\n'),
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
  /** Present only for languages with top-level declaration support (TS). */
  declQuery?: Parser.Query;
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
      const declSrc = DECL_QUERY[a.lang];
      langs.set(a.lang, {
        language,
        rosterQuery: language.query(ROSTER_QUERY[a.lang]),
        keyLineQuery: language.query(a.keyLineQuery),
        declQuery: declSrc ? language.query(declSrc) : undefined,
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
    const decls = compiled.declQuery ? extractDecls(compiled.declQuery, tree, roster) : [];
    return { roster, keyLines, decls };
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

/** Coarse kind of a top-level declaration node (for the explain hotspot label). */
function declKind(node: Parser.SyntaxNode): DeclSpan['kind'] {
  switch (node.type) {
    case 'type_alias_declaration':
      return 'type';
    case 'interface_declaration':
      return 'interface';
    case 'enum_declaration':
      return 'enum';
    case 'lexical_declaration':
      return node.firstChild?.text === 'let' ? 'let' : 'const';
    default:
      return 'const';
  }
}

/** Top-level declarations as manual-explain entries (S-TS-3). Skips any whose start
 *  line coincides with a roster function — a top-level `const f = () => {}` is a
 *  function capsule, not a plain declaration. Sorted by start line, deduped by id. */
function extractDecls(
  query: Parser.Query,
  tree: Parser.Tree,
  roster: FunctionSpan[],
): DeclSpan[] {
  const rosterStarts = new Set(roster.map((r) => r.lineRange[0]));
  const byId = new Map<string, DeclSpan>();
  for (const m of query.matches(tree.rootNode)) {
    const decl = m.captures.find((c) => c.name === 'decl')?.node;
    const name = m.captures.find((c) => c.name === 'name')?.node;
    if (!decl || !name) continue;
    const start = decl.startPosition.row + 1;
    if (rosterStarts.has(start)) continue; // function-valued const: already a capsule
    const id = `${name.text}#${start}`;
    if (byId.has(id)) continue;
    byId.set(id, {
      id,
      name: name.text,
      kind: declKind(decl),
      lineRange: [start, decl.endPosition.row + 1],
    });
  }
  return [...byId.values()].sort((a, b) => a.lineRange[0] - b.lineRange[0]);
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
