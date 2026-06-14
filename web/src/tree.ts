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
  children: TreeEntry[]
}

export type TreeEntry = TreeDir | TreeFile

/** Build a nested directory tree from the flat FileNode[] the backend returns. */
export function buildTree(files: FileNode[]): TreeEntry[] {
  const root: TreeDir = { kind: 'dir', name: '', children: [] }

  for (const f of files) {
    const parts = f.path.split('/')
    let dir = root
    for (let i = 0; i < parts.length - 1; i++) {
      const seg = parts[i]
      let next = dir.children.find(
        (c): c is TreeDir => c.kind === 'dir' && c.name === seg,
      )
      if (!next) {
        next = { kind: 'dir', name: seg, children: [] }
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
