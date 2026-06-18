//! ProjectReader — lists the project file tree (L0) and reads single source files.
//!
//! S1 scope: pure file IO. No graph, no LLM, no cache. The reader holds a
//! canonicalized project root and refuses any read that resolves outside it.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use walkdir::{DirEntry, WalkDir};

/// Directories that are never part of the readable tree (VCS / build / tooling noise).
/// `.understand-anything` and `.fluid` are data Fluid consumes, not source to browse.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    ".understand-anything",
    ".fluid",
    ".claude",
    ".idea",
    ".vscode",
];

/// A single file in the L0 skeleton (mirrors the TS `FileNode` in 技术方案 §3).
#[derive(Debug, Serialize)]
pub struct FileNode {
    /// Project-relative path, always forward-slash separated.
    pub path: String,
    /// Bare file name.
    pub name: String,
    /// Coarse language tag for the frontend: "py" | "rs" | "ts" | "tsx" | "js" |
    /// "jsx" | "md" | "other".
    pub lang: &'static str,
}

/// Why a single-file read was refused.
#[derive(Debug)]
pub enum ReadErr {
    /// Path does not exist (or is not a regular file).
    NotFound,
    /// Path tried to escape the project root (traversal / absolute / symlink-out).
    Forbidden,
}

pub struct ProjectReader {
    root: PathBuf,
}

impl ProjectReader {
    /// Build a reader rooted at `root`. The root is canonicalized so later
    /// `starts_with` containment checks are sound, and must be a directory
    /// (U3 Open Folder feeds arbitrary user paths here).
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "project root is not a directory",
            ));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Walk the project and return every regular file outside the skip dirs,
    /// sorted by relative path for stable output.
    pub fn list_files(&self) -> Vec<FileNode> {
        let mut out = Vec::new();
        let walker = WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| !is_skipped_dir(e));

        for entry in walker.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let abs = entry.path();
            let Ok(rel) = abs.strip_prefix(&self.root) else {
                continue;
            };
            out.push(FileNode {
                path: rel_to_unix(rel),
                name: entry.file_name().to_string_lossy().into_owned(),
                lang: lang_of(abs),
            });
        }

        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Read a single file by its project-relative path. Decodes as UTF-8,
    /// falling back to lossy so non-UTF-8 source still returns (read-only view).
    pub fn read_file(&self, rel: &str) -> Result<String, ReadErr> {
        let safe = self.resolve(rel)?;
        let bytes = std::fs::read(&safe).map_err(|_| ReadErr::NotFound)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Resolve a relative path to an absolute one, guaranteed inside the root.
    /// Rejects absolute paths, `..` components, and anything that canonicalizes
    /// outside the root (e.g. a symlink pointing away).
    fn resolve(&self, rel: &str) -> Result<PathBuf, ReadErr> {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(ReadErr::Forbidden);
        }
        for component in rel_path.components() {
            match component {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                    return Err(ReadErr::Forbidden);
                }
            }
        }

        let joined = self.root.join(rel_path);
        let canon = joined.canonicalize().map_err(|_| ReadErr::NotFound)?;
        if !canon.starts_with(&self.root) {
            return Err(ReadErr::Forbidden);
        }
        if !canon.is_file() {
            return Err(ReadErr::NotFound);
        }
        Ok(canon)
    }
}

fn is_skipped_dir(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    entry
        .file_name()
        .to_str()
        .map(|name| SKIP_DIRS.contains(&name))
        .unwrap_or(false)
}

fn rel_to_unix(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn lang_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("py") => "py",
        Some("rs") => "rs",
        // TS/JS family: distinct tags so the frontend can pick the right CM6
        // highlighter (typescript / jsx flavor). These are presentation tags only
        // — ghost-annotation generation stays gated to py/rs (isParserLang) until
        // a TS key-line query lands, so a .ts file opens highlighted but read-only.
        Some("ts") | Some("mts") | Some("cts") => "ts",
        Some("tsx") => "tsx",
        Some("js") | Some("mjs") | Some("cjs") => "js",
        Some("jsx") => "jsx",
        // Markdown gets its own tag so the frontend renders it as a formatted
        // document (Document Render View) instead of plain source. Not a
        // generation language — md never enters the capsule/ghost pipeline.
        Some("md") | Some("markdown") => "md",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build a reader over a temp project containing `src/a.py`.
    fn temp_reader() -> (tempdir_guard::TempDir, ProjectReader) {
        let dir = tempdir_guard::TempDir::new();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a.py"), "print('hi')\n").unwrap();
        let reader = ProjectReader::new(dir.path().to_path_buf()).unwrap();
        (dir, reader)
    }

    #[test]
    fn reads_a_normal_relative_file() {
        let (_dir, reader) = temp_reader();
        assert_eq!(reader.read_file("src/a.py").unwrap(), "print('hi')\n");
    }

    #[test]
    fn rejects_parent_traversal() {
        let (_dir, reader) = temp_reader();
        assert!(matches!(
            reader.read_file("../secret.txt"),
            Err(ReadErr::Forbidden)
        ));
        assert!(matches!(
            reader.read_file("src/../../secret.txt"),
            Err(ReadErr::Forbidden)
        ));
    }

    #[test]
    fn rejects_absolute_path() {
        let (_dir, reader) = temp_reader();
        let abs = if cfg!(windows) {
            "C:/Windows/System32/drivers/etc/hosts"
        } else {
            "/etc/passwd"
        };
        assert!(matches!(reader.read_file(abs), Err(ReadErr::Forbidden)));
    }

    #[test]
    fn missing_file_is_not_found() {
        let (_dir, reader) = temp_reader();
        assert!(matches!(reader.read_file("src/nope.py"), Err(ReadErr::NotFound)));
    }

    #[test]
    fn tags_markdown_as_md() {
        // .md / .markdown get the "md" tag (Document Render View); other
        // extensions stay "other"; code langs unchanged.
        assert_eq!(lang_of(Path::new("README.md")), "md");
        assert_eq!(lang_of(Path::new("docs/guide.markdown")), "md");
        assert_eq!(lang_of(Path::new("a.py")), "py");
        assert_eq!(lang_of(Path::new("notes.txt")), "other");
    }

    #[test]
    fn tags_ts_js_family_for_highlighting() {
        // TS/JS family map to flavor-specific tags (highlighting only — still
        // gated out of generation by the frontend until a TS key-line query lands).
        assert_eq!(lang_of(Path::new("src/app.ts")), "ts");
        assert_eq!(lang_of(Path::new("a.mts")), "ts");
        assert_eq!(lang_of(Path::new("ui/Button.tsx")), "tsx");
        assert_eq!(lang_of(Path::new("legacy.js")), "js");
        assert_eq!(lang_of(Path::new("esm.mjs")), "js");
        assert_eq!(lang_of(Path::new("Widget.jsx")), "jsx");
        // Unchanged neighbors.
        assert_eq!(lang_of(Path::new("main.rs")), "rs");
        assert_eq!(lang_of(Path::new("notes.txt")), "other");
    }

    #[test]
    fn lists_files_skips_noise_dirs() {
        let dir = tempdir_guard::TempDir::new();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join("pkg/m.py"), "x = 1\n").unwrap();
        fs::write(dir.path().join("__pycache__/m.cpython.pyc"), "junk").unwrap();
        fs::write(dir.path().join(".git/config"), "[core]").unwrap();
        let reader = ProjectReader::new(dir.path().to_path_buf()).unwrap();

        let files = reader.list_files();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["pkg/m.py"]);
        assert_eq!(files[0].lang, "py");
    }

    /// Minimal self-cleaning temp dir (avoids an external dev-dependency for S1).
    mod tempdir_guard {
        use std::path::{Path, PathBuf};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> Self {
                let unique = format!(
                    "fluid-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let path = std::env::temp_dir().join(unique);
                std::fs::create_dir_all(&path).unwrap();
                TempDir(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
