// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Application preferences, separate from label contents and undo history.

use std::path::{Path, PathBuf};

const LIMIT: usize = 10;
const STORAGE_KEY: &str = "recent_files";

#[derive(Default)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let paths: Vec<PathBuf> = storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default();
        let mut recent = Self::default();
        // Do not probe removable disks or network shares while starting the app.
        for path in paths {
            if path.is_absolute() && !recent.paths.contains(&path) {
                recent.paths.push(path);
                if recent.paths.len() == LIMIT {
                    break;
                }
            }
        }
        recent
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        // RON paths require Unicode. One native non-Unicode name must not
        // prevent the other entries from being saved.
        let paths: Vec<_> = self
            .paths
            .iter()
            .filter(|path| path.to_str().is_some())
            .collect();
        eframe::set_value(storage, STORAGE_KEY, &paths);
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn remember(&mut self, path: &Path) {
        // Only called after a successful open or save, when the file exists.
        // Resolve relative paths and aliases once, never while drawing menus.
        let Ok(path) = path.canonicalize().or_else(|_| std::path::absolute(path)) else {
            return;
        };
        self.paths.retain(|entry| entry != &path);
        self.paths.insert(0, path);
        self.paths.truncate(LIMIT);
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|entry| entry != path);
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    pub fn caption(&self, path: &Path) -> String {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let duplicate = self
            .paths
            .iter()
            .filter(|entry| entry.file_name() == path.file_name())
            .count()
            > 1;
        if duplicate && let Some(parent) = path.parent() {
            format!("{name} ({})", display_path(parent))
        } else {
            name.into_owned()
        }
    }
}

pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.to_owned();
        }
    }
    text.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Storage(HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn usage_order_aliases_and_restart_keep_one_entry_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("配线架 A.ptl");
        let b = dir.path().join("B.ptl");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let mut recent = RecentFiles::default();
        recent.remember(&a);
        recent.remember(&b);
        recent.remember(&dir.path().join(".").join("配线架 A.ptl"));
        assert_eq!(
            recent.paths(),
            &[a.canonicalize().unwrap(), b.canonicalize().unwrap()]
        );
        let mut storage = Storage::default();
        recent.save(&mut storage);
        assert_eq!(RecentFiles::load(Some(&storage)).paths(), recent.paths());
        recent.clear();
        recent.save(&mut storage);
        assert!(RecentFiles::load(Some(&storage)).paths().is_empty());
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn restore_is_bounded_and_keeps_unavailable_files_without_resolving_them() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = (0..20)
            .map(|i| dir.path().join(format!("{i}.ptl")))
            .collect();
        let mut stored = vec![PathBuf::from("relative.ptl"), paths[0].clone()];
        stored.extend(paths.clone());
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &stored);
        let mut recent = RecentFiles::load(Some(&storage));
        assert_eq!(recent.paths(), &paths[..LIMIT]);
        recent.remove(&paths[0]);
        assert_eq!(recent.paths(), &paths[1..LIMIT]);
        storage
            .0
            .insert(STORAGE_KEY.into(), "invalid preferences".into());
        assert!(RecentFiles::load(Some(&storage)).paths().is_empty());
    }

    #[test]
    fn identical_names_show_their_directories() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("rack-a/label.ptl");
        let b = dir.path().join("rack-b/label.ptl");
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &vec![a.clone(), b.clone()]);
        let recent = RecentFiles::load(Some(&storage));
        assert!(recent.caption(&a).contains("rack-a"));
        assert!(recent.caption(&b).contains("rack-b"));
    }
}
