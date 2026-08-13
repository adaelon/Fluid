// S-WTREE-1 deterministic checks. Run with:
//   node web/scripts/workspace-tree-check.ts
// The pure directory-state core and Vue ownership wiring are checked without a browser.

import { readFileSync } from 'node:fs'
import type { FileNode } from '../src/api.ts'
import {
  buildTree,
  restoreExpandedDirectories,
  setDirectoryExpanded,
  type TreeDir,
  type TreeEntry,
} from '../src/tree.ts'

let failures = 0
function check(label: string, condition: boolean): void {
  if (condition) console.log(`  PASS  ${label}`)
  else {
    console.error(`  FAIL  ${label}`)
    failures++
  }
}

function file(path: string): FileNode {
  return { path, name: path.split('/').pop() ?? path, lang: 'rs' }
}

function directories(entries: readonly TreeEntry[]): TreeDir[] {
  const result: TreeDir[] = []
  for (const entry of entries) {
    if (entry.kind !== 'dir') continue
    result.push(entry, ...directories(entry.children))
  }
  return result
}

const files = [
  file('alpha/shared/lib.rs'),
  file('alpha/solo.rs'),
  file('beta/shared/lib.rs'),
  file('beta/nested/shared/deep.rs'),
  file('README.rs'),
]

console.log('=== complete directory identity ===')
const tree = buildTree(files)
const directoryPaths = directories(tree).map((entry) => entry.path)
check(
  'every directory receives its complete project-relative path',
  directoryPaths.join(',')
    === 'alpha,alpha/shared,beta,beta/nested,beta/nested/shared,beta/shared',
)
check(
  'nested same-name directories have distinct persistent identities',
  new Set(directoryPaths.filter((path) => path.endsWith('shared'))).size === 3,
)

console.log('\n=== immutable single-directory updates ===')
const initial = new Set(['alpha', 'alpha/shared', 'beta/shared'])
const collapsed = setDirectoryExpanded(initial, {
  path: 'alpha/shared',
  expanded: false,
})
check(
  'collapsing one same-name directory changes only that complete path',
  [...collapsed].join(',') === 'alpha,beta/shared',
)
check(
  'the reducer does not mutate the previous set',
  [...initial].join(',') === 'alpha,alpha/shared,beta/shared',
)
const expanded = setDirectoryExpanded(collapsed, {
  path: 'beta/nested/shared',
  expanded: true,
})
check(
  'expanding a directory preserves every unrelated entry',
  [...expanded].join(',') === 'alpha,beta/shared,beta/nested/shared',
)
check(
  'an idempotent update preserves the current set identity',
  setDirectoryExpanded(expanded, { path: 'beta/shared', expanded: true }) === expanded,
)
check(
  'the reducer ignores a syntactically illegal event path',
  setDirectoryExpanded(expanded, { path: '../escape', expanded: true }) === expanded,
)

console.log('\n=== persisted-set projection and replacement ===')
const restored = restoreExpandedDirectories(tree, [
  'alpha/shared',
  'alpha/shared/lib.rs',
  'missing',
  '../escape',
  '/absolute',
  'beta/shared/',
  'beta/nested/shared',
  'alpha/shared',
])
check(
  'only real directory paths survive restoration',
  [...restored.expandedDirectories].join(',') === 'alpha/shared,beta/nested/shared',
)
check(
  'file, stale, traversal, absolute and malformed paths remain observable',
  restored.skippedDirectories.join(',')
    === 'alpha/shared/lib.rs,missing,../escape,/absolute,beta/shared/',
)
const rebuilt = restoreExpandedDirectories(buildTree([...files]), [
  ...restored.expandedDirectories,
])
check(
  'rebuilding an equivalent tree projects the same expanded set',
  [...rebuilt.expandedDirectories].join(',')
    === [...restored.expandedDirectories].join(','),
)
const poisonedTree = restoreExpandedDirectories(
  buildTree([...files, file('../escape/inside.rs')]),
  ['../escape'],
)
check(
  'illegal paths are rejected even if a malformed tree payload contains them',
  poisonedTree.expandedDirectories.size === 0
    && poisonedTree.skippedDirectories.join(',') === '../escape',
)
const replacement = restoreExpandedDirectories(buildTree([
  file('other/only.rs'),
]), ['other'])
check(
  'a project switch returns a complete replacement without prior-project paths',
  [...replacement.expandedDirectories].join(',') === 'other'
    && !replacement.expandedDirectories.has('alpha/shared'),
)

console.log('\n=== controlled Vue ownership wiring ===')
const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const fileTreeSource = readFileSync(new URL('../src/FileTree.vue', import.meta.url), 'utf8')
const treeNodeSource = readFileSync(new URL('../src/TreeNode.vue', import.meta.url), 'utf8')
check(
  'App owns and replaces the one expanded-directory set from each snapshot',
  appSource.includes('const expandedDirectories = ref<ReadonlySet<string>>(new Set())')
    && appSource.includes('restoreExpandedDirectories(tree, snapshot?.expandedDirectories ?? [])')
    && appSource.includes('expandedDirectories.value = restored.expandedDirectories'),
)
check(
  'App passes controlled state down and consumes one structured update event',
  appSource.includes(':expanded-directories="expandedDirectories"')
    && appSource.includes('@set-directory-expanded="setDirectoryExpanded"'),
)
check(
  'FileTree keys directories by complete path and forwards controlled updates',
  fileTreeSource.includes("'d:' + entry.path")
    && fileTreeSource.includes(':expanded-directories="expandedDirectories"')
    && fileTreeSource.includes('@set-directory-expanded'),
)
check(
  'recursive nodes project the set and never keep a private open ref',
  treeNodeSource.includes('expandedDirectories.has(props.entry.path)')
    && treeNodeSource.includes("emit('set-directory-expanded'")
    && !treeNodeSource.includes('const open = ref('),
)

if (failures > 0) {
  console.error(`\n${failures} FAILED`)
  process.exit(1)
}
console.log('\nAll workspace-tree checks passed.')
