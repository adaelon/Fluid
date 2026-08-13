import type { FileNode } from './api'

export interface TreeFile {
  kind: 'file'
  name: string
  path: string
  lang: string
}

export interface TreeDir {
  kind: 'dir'
  name: string
  path: string
  children: TreeEntry[]
}

export type TreeEntry = TreeDir | TreeFile

export interface DirectoryExpansionChange {
  path: string
  expanded: boolean
}

export interface RestoredExpandedDirectories {
  expandedDirectories: ReadonlySet<string>
  skippedDirectories: string[]
}

/** Build a nested directory tree from the flat FileNode[] the backend returns. */
export function buildTree(files: FileNode[]): TreeEntry[] {
  const root: TreeDir = { kind: 'dir', name: '', path: '', children: [] }

  for (const f of files) {
    const parts = f.path.split('/')
    let dir = root
    for (let i = 0; i < parts.length - 1; i++) {
      const seg = parts[i]
      const path = dir.path ? `${dir.path}/${seg}` : seg
      let next = dir.children.find(
        (c): c is TreeDir => c.kind === 'dir' && c.path === path,
      )
      if (!next) {
        next = { kind: 'dir', name: seg, path, children: [] }
        dir.children.push(next)
      }
      dir = next
    }
    dir.children.push({
      kind: 'file',
      name: parts[parts.length - 1],
      path: f.path,
      lang: f.lang,
    })
  }

  sortDir(root)
  return root.children
}

/** Replace one directory's controlled expansion state without mutating the
 * previous set. Idempotent updates preserve the existing set identity. */
export function setDirectoryExpanded(
  expandedDirectories: ReadonlySet<string>,
  change: DirectoryExpansionChange,
): ReadonlySet<string> {
  if (!isCanonicalRelativePath(change.path)) return expandedDirectories
  if (expandedDirectories.has(change.path) === change.expanded) {
    return expandedDirectories
  }

  const next = new Set(expandedDirectories)
  if (change.expanded) next.add(change.path)
  else next.delete(change.path)
  return next
}

/** Project a persisted directory set onto the current tree. Only complete paths
 * belonging to real directories survive; duplicates are harmless and every
 * rejected path remains available to the later warning layer. */
export function restoreExpandedDirectories(
  tree: readonly TreeEntry[],
  persistedDirectories: readonly string[],
): RestoredExpandedDirectories {
  const available = collectDirectoryPaths(tree)
  const expandedDirectories = new Set<string>()
  const skippedDirectories: string[] = []

  for (const path of persistedDirectories) {
    if (isCanonicalRelativePath(path) && available.has(path)) {
      expandedDirectories.add(path)
    }
    else skippedDirectories.push(path)
  }

  return { expandedDirectories, skippedDirectories }
}

function isCanonicalRelativePath(path: string): boolean {
  if (
    path.length === 0
    || path.startsWith('/')
    || path.endsWith('/')
    || path.includes('\\')
    || path.includes('\0')
    || /^[A-Za-z]:/.test(path)
  ) return false

  return path.split('/').every(
    (segment) => segment.length > 0 && segment !== '.' && segment !== '..',
  )
}

function collectDirectoryPaths(entries: readonly TreeEntry[]): Set<string> {
  const paths = new Set<string>()
  for (const entry of entries) {
    if (entry.kind !== 'dir') continue
    paths.add(entry.path)
    for (const path of collectDirectoryPaths(entry.children)) paths.add(path)
  }
  return paths
}

/** Dirs before files; each group alphabetical. */
function sortDir(dir: TreeDir): void {
  dir.children.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === 'dir' ? -1 : 1
    return a.name.localeCompare(b.name)
  })
  for (const c of dir.children) {
    if (c.kind === 'dir') sortDir(c)
  }
}
