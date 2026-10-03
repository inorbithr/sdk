use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// The rendered surface: files by path relative to the output directory, and notes
/// for the person (an operation that was skipped, and why).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    files: BTreeMap<PathBuf, Vec<u8>>,
    notes: Vec<String>,
}

impl Files {
    /// No files yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a file.
    pub fn insert(&mut self, path: impl Into<PathBuf>, content: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), content.into());
    }

    /// Adds a note for the person.
    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// The files, in path order.
    pub fn iter(&self) -> impl Iterator<Item = (&Path, &[u8])> {
        self.files.iter().map(|(p, c)| (p.as_path(), c.as_slice()))
    }

    /// The notes, in order.
    #[must_use]
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// How many files there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether there is no file.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Writes every file under `out`, creating directories as needed. With `clean`,
    /// what is under `out` first goes, so a file from an earlier run never lingers.
    ///
    /// # Errors
    ///
    /// The file system's.
    pub fn write(&self, out: &Path, clean: bool) -> io::Result<()> {
        if clean && out.exists() {
            std::fs::remove_dir_all(out)?;
        }
        std::fs::create_dir_all(out)?;
        for (path, content) in &self.files {
            let target = out.join(path);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(target, content)?;
        }
        Ok(())
    }

    /// The paths under `out` whose content differs from these files, or which are
    /// missing or extra there: what `iohr sdk check --files` reports.
    ///
    /// # Errors
    ///
    /// The file system's.
    pub fn diff(&self, out: &Path) -> io::Result<Vec<String>> {
        let mut changed = Vec::new();
        for (path, content) in &self.files {
            match std::fs::read(out.join(path)) {
                Ok(existing) if existing == *content => {}
                Ok(_) => changed.push(format!("~ {}", path.display())),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    changed.push(format!("+ {}", path.display()));
                }
                Err(e) => return Err(e),
            }
        }
        let mut extra = Vec::new();
        walk(out, out, &mut extra)?;
        for path in extra {
            if !self.files.contains_key(&path) {
                changed.push(format!("- {}", path.display()));
            }
        }
        changed.sort();
        Ok(changed)
    }
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            found.push(rel.to_path_buf());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Files;

    #[test]
    fn writes_cleans_and_diffs() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("surface");
        let mut a = Files::new();
        a.insert("mod.rs", "one");
        a.insert("models.rs", "two");
        a.write(&out, true).unwrap();
        assert_eq!(a.diff(&out).unwrap(), Vec::<String>::new());
        std::fs::write(out.join("stale.rs"), "x").unwrap();
        let mut b = Files::new();
        b.insert("mod.rs", "one");
        b.insert("models.rs", "changed");
        b.insert("ops.rs", "new");
        assert_eq!(
            b.diff(&out).unwrap(),
            ["+ ops.rs", "- stale.rs", "~ models.rs"]
        );
        b.write(&out, true).unwrap();
        assert!(!out.join("stale.rs").exists());
        assert_eq!(b.diff(&out).unwrap(), Vec::<String>::new());
    }
}
