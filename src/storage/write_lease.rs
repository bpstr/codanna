//! Advisory ownership for a whole code-index write lifecycle, outside the index
//! directory so a force rebuild cannot unlink a live owner's lock. Reentrant
//! within a process; actual same-process mutations still use the runtime gate.
use crate::{IndexError, IndexResult};
use fs4::fs_std::FileExt;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug)]
pub struct CodeWriteLease {
    _file: File,
}

impl CodeWriteLease {
    pub fn try_acquire(index: &Path) -> IndexResult<Option<Arc<Self>>> {
        let absolute = match index.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::path::absolute(index)?,
            Err(error) => return Err(error.into()),
        };
        let parent = absolute.parent().ok_or_else(|| error("Code index has no parent"))?;
        fs::create_dir_all(parent)?;
        let parent = parent.canonicalize()?;
        let mut name = OsString::from(".");
        name.push(absolute.file_name().ok_or_else(|| error("Code index has no name"))?);
        name.push(".codanna-writer.lock");
        let path = parent.join(name);
        static HELD: OnceLock<Mutex<HashMap<PathBuf, Weak<CodeWriteLease>>>> = OnceLock::new();
        let mut held = HELD.get_or_init(Mutex::default).lock().map_err(|_| IndexError::MutexPoisoned)?;
        held.retain(|_, lease| lease.strong_count() > 0);
        if let Some(lease) = held.get(&path).and_then(Weak::upgrade) {
            return Ok(Some(lease));
        }
        match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() => return Err(error("Code writer lock must be a regular file")),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        let named = fs::symlink_metadata(&path)?;
        let opened = file.metadata()?;
        if !named.is_file() || !opened.is_file() {
            return Err(error("Code writer lock changed type during acquisition"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if named.dev() != opened.dev() || named.ino() != opened.ino() {
                return Err(error("Code writer lock changed identity during acquisition"));
            }
        }
        if !FileExt::try_lock_exclusive(&file)? {
            return Ok(None);
        }
        let lease = Arc::new(Self { _file: file });
        held.insert(path, Arc::downgrade(&lease));
        Ok(Some(lease))
    }

    pub fn acquire(index: &Path) -> IndexResult<Arc<Self>> {
        Self::try_acquire(index)?.ok_or_else(|| error(
            "Code index already has an active writer or workspace watcher. Close that workspace connection before an explicit rebuild; no existing index was changed."
        ))
    }
}
fn error(message: &str) -> IndexError { IndexError::General(message.into()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardening_workspace_write_lease_survives_index_replacement_and_is_reentrant() {
        let temp = tempfile::tempdir().unwrap();
        let index = temp.path().join("index");
        fs::create_dir(&index).unwrap();
        let first = CodeWriteLease::acquire(&index).unwrap();
        let second = CodeWriteLease::acquire(&index).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        fs::remove_dir(&index).unwrap();
        fs::create_dir(&index).unwrap();
        let file = OpenOptions::new().read(true).write(true).open(temp.path().join(".index.codanna-writer.lock")).unwrap();
        assert!(!FileExt::try_lock_exclusive(&file).unwrap());
        drop(first);
        assert!(!FileExt::try_lock_exclusive(&file).unwrap());
        drop(second);
        assert!(FileExt::try_lock_exclusive(&file).unwrap());
    }
}
