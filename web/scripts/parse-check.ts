// S4 validation (B2, deterministic). Runs the real FluidParser against two real
// samples and prints roster + key lines for manual cross-check.
//   node scripts/parse-check.ts        (Node 24 strips TS types natively)
// Grammars come from the tree-sitter-wasms dependency; queries from the in-repo
// .scm files — the exact assets the browser bootstrap will feed in S7.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { FluidParser, type LangAsset, type ParserLang } from '../src/parser/index.ts';

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, '..');
const grammar = (n: string) => readFileSync(join(web, 'node_modules/tree-sitter-wasms/out', `tree-sitter-${n}.wasm`));
const scm = (n: string) => readFileSync(join(web, 'src/parser/keyline-queries', `${n}.scm`), 'utf8');

const assets: LangAsset[] = [
  { lang: 'py', grammarWasm: grammar('python'), keyLineQuery: scm('python') },
  { lang: 'rs', grammarWasm: grammar('rust'), keyLineQuery: scm('rust') },
  { lang: 'ts', grammarWasm: grammar('typescript'), keyLineQuery: scm('typescript') },
];

const samples: Array<{ label: string; lang: ParserLang; path: string }> = [
  { label: 'alphaGPT/execution/config.py', lang: 'py', path: 'E:/allwork/download/agent/alphaGPT/execution/config.py' },
  { label: 'fluid-server/src/project_reader.rs', lang: 'rs', path: 'E:/allwork/download/agent/Fluid/crates/fluid-server/src/project_reader.rs' },
];

const parser = await FluidParser.create(assets);

for (const s of samples) {
  const { roster, keyLines } = parser.parse(s.lang, readFileSync(s.path, 'utf8'));
  console.log(`\n===== ${s.label} =====  (${roster.length} functions)`);
  for (const fn of roster) {
    const ks = keyLines.get(fn.id) ?? [];
    console.log(`  ${fn.name}  L${fn.lineRange[0]}-${fn.lineRange[1]}  keyLines=[${ks.join(',')}]`);
  }
}

// --- TypeScript: synthetic sample with known constructs, asserted (B2 gate) ---
// Covers every roster shape (function decl, const arrow, const function-expr, class
// method, class-field arrow, nested arrow) and key-line kinds (lexical_declaration,
// re-assignment, value return, throw, if, statement call, awaited call).
const TS_SAMPLE = `import { dep } from './dep'

const MAX = 100
export const API_URL = 'https://x'
type Props = { a: number }
export interface Opts { b: string }
enum Color { Red, Green }

export function alpha(n: number): number {
  const x = n * 2
  if (x > 10) {
    return x
  }
  return 0
}

const beta = (s: string): string => {
  doThing()
  return s.trim()
}

const gamma = function (a: number) {
  let total = 0
  total += a
  return total
}

class Widget {
  count = 0
  handle = async (): Promise<void> => {
    await save()
    this.count += 1
  }
  render(): string {
    throw new Error('x')
  }
}
`;

const tsParse = parser.parse('ts', TS_SAMPLE);
console.log(`\n===== TS synthetic =====  (${tsParse.roster.length} functions)`);
for (const fn of tsParse.roster) {
  const ks = tsParse.keyLines.get(fn.id) ?? [];
  console.log(`  ${fn.name}  L${fn.lineRange[0]}-${fn.lineRange[1]}  keyLines=[${ks.join(',')}]`);
}

// 1-based line number of the first line containing `needle`.
const lineOf = (needle: string): number =>
  TS_SAMPLE.split('\n').findIndex((l) => l.includes(needle)) + 1;

// Resolve a function's key-line set by name (roster ids are name#startLine).
const keyLinesOf = (name: string): number[] => {
  const fn = tsParse.roster.find((r) => r.name === name);
  if (!fn) return [];
  return tsParse.keyLines.get(fn.id) ?? [];
};

const failures: string[] = [];
const expect = (cond: boolean, msg: string) => { if (!cond) failures.push(msg); };

// Roster: exactly the six function shapes; class `Widget` and the plain `count`
// field are NOT functions.
const names = tsParse.roster.map((r) => r.name).sort();
expect(
  JSON.stringify(names) === JSON.stringify(['alpha', 'beta', 'gamma', 'handle', 'render']),
  `roster names = ${JSON.stringify(names)} (want alpha,beta,gamma,handle,render)`,
);

// Key lines land in the right host (innermostHost), by content not raw numbers.
expect(keyLinesOf('alpha').includes(lineOf('const x = n * 2')), 'alpha: const init');
expect(keyLinesOf('alpha').includes(lineOf('if (x > 10)')), 'alpha: if head');
expect(keyLinesOf('alpha').includes(lineOf('return x')), 'alpha: value return');
expect(keyLinesOf('beta').includes(lineOf('doThing()')), 'beta: statement call');
expect(keyLinesOf('beta').includes(lineOf('return s.trim()')), 'beta: value return');
expect(keyLinesOf('gamma').includes(lineOf('total += a')), 'gamma: compound assign');
expect(keyLinesOf('handle').includes(lineOf('await save()')), 'handle: awaited call');
expect(keyLinesOf('handle').includes(lineOf('this.count += 1')), 'handle: compound assign');
expect(keyLinesOf('render').includes(lineOf("throw new Error('x')")), 'render: throw');

// Top-level declarations (S-TS-3): the manual-explain discovery list. Function-
// valued consts (beta/gamma) and class members (handle/render) are NOT decls.
console.log(`\n--- TS top-level decls (${tsParse.decls.length}) ---`);
for (const d of tsParse.decls) console.log(`  ${d.kind} ${d.name}  L${d.lineRange[0]}-${d.lineRange[1]}`);
const declSig = tsParse.decls.map((d) => `${d.kind}:${d.name}`).sort();
expect(
  JSON.stringify(declSig) ===
    JSON.stringify(['const:API_URL', 'const:MAX', 'enum:Color', 'interface:Opts', 'type:Props']),
  `decls = ${JSON.stringify(declSig)} (want MAX/API_URL const, Props type, Opts interface, Color enum)`,
);
expect(!tsParse.decls.some((d) => d.name === 'beta' || d.name === 'gamma'),
  'function-valued top-level const must NOT be a decl (it is a roster capsule)');
expect(!tsParse.decls.some((d) => d.name === 'handle' || d.name === 'render'),
  'class members must NOT be top-level decls');

if (failures.length) {
  console.error(`\n✗ TS assertions FAILED (${failures.length}):`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log('\n✓ TS roster + key-line assertions passed');
