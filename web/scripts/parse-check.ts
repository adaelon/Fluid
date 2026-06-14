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
