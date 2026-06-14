// Browser bootstrap for FluidParser (the asset assembly S4 deferred to S7).
//
// Vite serves the grammar/core wasm as URLs (`?url`) and the key-line queries as
// raw text (`?raw`); we fetch the wasm bytes and hand everything to the
// environment-agnostic FluidParser.create. One lazily-initialized singleton — the
// WASM runtime + compiled queries are expensive, so all files reuse the parser.

import coreWasmUrl from 'web-tree-sitter/tree-sitter.wasm?url'
import pyWasmUrl from 'tree-sitter-wasms/out/tree-sitter-python.wasm?url'
import rsWasmUrl from 'tree-sitter-wasms/out/tree-sitter-rust.wasm?url'
import pyQuery from './keyline-queries/python.scm?raw'
import rsQuery from './keyline-queries/rust.scm?raw'
import { FluidParser, type LangAsset } from './index.ts'

let singleton: Promise<FluidParser> | null = null

async function fetchBytes(url: string): Promise<Uint8Array> {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`parser asset ${url} -> ${res.status}`)
  return new Uint8Array(await res.arrayBuffer())
}

/** Initialize (once) and return the shared parser for Python + Rust. */
export function getParser(): Promise<FluidParser> {
  if (!singleton) {
    singleton = (async () => {
      const [py, rs] = await Promise.all([fetchBytes(pyWasmUrl), fetchBytes(rsWasmUrl)])
      const assets: LangAsset[] = [
        { lang: 'py', grammarWasm: py, keyLineQuery: pyQuery },
        { lang: 'rs', grammarWasm: rs, keyLineQuery: rsQuery },
      ]
      // locateFile points web-tree-sitter's Emscripten loader at the core wasm URL.
      return FluidParser.create(assets, { locateFile: () => coreWasmUrl })
    })()
  }
  return singleton
}
