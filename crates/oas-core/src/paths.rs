//! The on-disk layout of the app's data directory.
//!
//! This module only *computes* paths by joining `PathBuf`s — it never creates
//! or reads files, so it stays I/O-free and cross-platform. Resolving the
//! actual root (via the OS app-data directory or a Tauri path API) is the job
//! of the layer above; it passes the root in here.
//!
//! All joins use `PathBuf`, never string concatenation, so paths containing
//! spaces or non-ASCII characters are handled correctly on every platform.

use std::path::{Path, PathBuf};

/// Resolves the standard sub-paths under a given app-data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
    /// Set when the user moved their models out of the app data folder.
    models_dir: Option<PathBuf>,
}

impl Paths {
    /// Build the layout rooted at `root` (e.g. `~/Library/Application Support/
    /// SynthSpeech` on macOS, resolved by the caller).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Paths {
            root: root.into(),
            models_dir: None,
        }
    }

    /// The same layout with models kept in `dir` instead of under the root.
    pub fn with_models_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.models_dir = Some(dir.into());
        self
    }

    /// The app-data root itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory holding downloaded model weights.
    pub fn models_dir(&self) -> PathBuf {
        self.models_dir
            .clone()
            .unwrap_or_else(|| self.root.join("models"))
    }

    /// Directory for rolling log files.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Directory for temporary files (in-progress downloads, scratch).
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Directory for a cached/refreshed catalog copy.
    pub fn catalog_dir(&self) -> PathBuf {
        self.root.join("catalog")
    }

    /// The main SQLite database file.
    pub fn database_file(&self) -> PathBuf {
        self.root.join("oas.sqlite3")
    }

    /// The directory that should hold a specific model's files.
    pub fn model_dir(&self, model_id: &crate::ModelId) -> PathBuf {
        self.models_dir().join(model_id.as_str())
    }

    /// Every directory that should exist for the app to run. The caller
    /// creates these (this crate does no I/O).
    pub fn required_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.root.clone(),
            self.models_dir(),
            self.logs_dir(),
            self.tmp_dir(),
            self.catalog_dir(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relocated_models_dir_leaves_the_rest_in_place() {
        let paths = Paths::new("/data").with_models_dir("/Volumes/Big Disk/modèles");
        assert_eq!(
            paths.models_dir(),
            PathBuf::from("/Volumes/Big Disk/modèles")
        );
        assert_eq!(paths.logs_dir(), PathBuf::from("/data").join("logs"));
    }
    use crate::ModelId;

    #[test]
    fn subpaths_are_under_root() {
        let p = Paths::new("/base");
        assert!(p.models_dir().starts_with("/base"));
        assert!(p.database_file().starts_with("/base"));
        assert_eq!(p.database_file().file_name().unwrap(), "oas.sqlite3");
    }

    #[test]
    fn handles_spaces_in_root() {
        // Mirrors the real macOS root "Application Support/SynthSpeech".
        let root = PathBuf::from("/Users/a b/Application Support/SynthSpeech");
        let p = Paths::new(&root);
        let models = p.models_dir();
        assert!(models.starts_with(&root));
        assert_eq!(models.file_name().unwrap(), "models");
        // The space-containing component is preserved intact, not split.
        assert!(models
            .components()
            .any(|c| c.as_os_str() == "Application Support"));
    }

    #[test]
    fn handles_unicode_in_root_and_model_id() {
        let root = PathBuf::from("/Users/练习/音频 studio");
        let p = Paths::new(&root);
        let id = ModelId::new("whisper-tiny").unwrap();
        let dir = p.model_dir(&id);
        assert!(dir.starts_with(&root));
        assert!(dir.ends_with("models/whisper-tiny"));
        assert!(dir.to_string_lossy().contains("音频 studio"));
    }

    #[test]
    fn required_dirs_lists_the_root_and_children() {
        let p = Paths::new("/base");
        let dirs = p.required_dirs();
        assert!(dirs.contains(&PathBuf::from("/base")));
        assert!(dirs.contains(&PathBuf::from("/base/models")));
        assert!(dirs.contains(&PathBuf::from("/base/logs")));
    }
}
