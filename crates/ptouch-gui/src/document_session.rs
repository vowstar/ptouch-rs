// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! File identity and saved content for the current editor document.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ptouch_render::document::{LabelDocument, LabelElement, LabelObject, ObjectContent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    Save,
    SaveAs,
}

/// The saved baseline is independent of edit revisions and undo positions.
/// Initialize it from the editor's actual initial document.
#[derive(Clone, Debug)]
pub struct DocumentSession {
    current_path: Option<PathBuf>,
    saved_document: LabelDocument,
    generation: u64,
}

impl DocumentSession {
    pub fn new(document: &LabelDocument) -> Self {
        Self {
            current_path: None,
            saved_document: saved_content(document),
            generation: 0,
        }
    }

    pub fn current_path(&self) -> Option<&Path> {
        self.current_path.as_deref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Start an untitled document with the supplied clean baseline.
    pub fn reset(&mut self, document: &LabelDocument) {
        let generation = self.generation.wrapping_add(1);
        *self = Self::new(document);
        self.generation = generation;
    }

    /// Call only after parsing and applying the opened document succeeded.
    pub fn opened(&mut self, path: PathBuf, document: &LabelDocument) {
        self.reset(document);
        self.current_path = Some(path);
    }

    /// Image render caches do not belong to the file's editable content.
    pub fn is_modified(&self, document: &LabelDocument) -> bool {
        let saved = &self.saved_document;
        document.version != saved.version
            || document.tape_width_mm != saved.tape_width_mm
            || document.dpi != saved.dpi
            || document.font_name != saved.font_name
            || document.font_margin != saved.font_margin
            || document.flip_h != saved.flip_h
            || document.flip_v != saved.flip_v
            || document.elements.len() != saved.elements.len()
            || document
                .elements
                .iter()
                .zip(&saved.elements)
                .any(|(current, saved)| !same_saved_element(current, saved))
    }

    pub fn needs_save_path(&self, kind: SaveKind) -> bool {
        kind == SaveKind::SaveAs || self.current_path.is_none()
    }

    /// A missing selected path represents a cancelled file dialog.
    /// Saving an existing file preserves its exact name and extension.
    pub fn save_target(&self, kind: SaveKind, selected_path: Option<&Path>) -> Option<PathBuf> {
        if kind == SaveKind::Save
            && let Some(path) = &self.current_path
        {
            return Some(path.clone());
        }
        selected_path.map(|path| {
            if path.extension().is_none() {
                path.with_extension("ptl")
            } else {
                path.to_path_buf()
            }
        })
    }

    /// Serialize and replace the file before updating its identity or baseline.
    /// Returns `Ok(None)` when the file dialog was cancelled.
    pub fn save(
        &mut self,
        kind: SaveKind,
        selected_path: Option<&Path>,
        document: &LabelDocument,
    ) -> Result<Option<PathBuf>, SaveError> {
        if self.needs_save_path(kind)
            && let Some(path) = selected_path
            && (path.as_os_str().is_empty() || path.file_name().is_none() || path.is_dir())
        {
            return Err(SaveError::Write {
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "Select a file name"),
            });
        }
        let Some(path) = self.save_target(kind, selected_path) else {
            return Ok(None);
        };
        let text = document
            .to_toml_string()
            .map_err(SaveError::InvalidDocument)?;
        write_atomic(&path, text.as_bytes()).map_err(|source| SaveError::Write {
            path: path.clone(),
            source,
        })?;
        self.opened(path.clone(), document);
        Ok(Some(path))
    }
}

#[derive(Debug)]
pub enum SaveError {
    InvalidDocument(ptouch_render::RenderError),
    Write { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDocument(error) => write!(formatter, "Cannot save this layout: {error}"),
            Self::Write { path, source } => {
                write!(formatter, "Cannot save {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidDocument(error) => Some(error),
            Self::Write { source, .. } => Some(source),
        }
    }
}

fn saved_content(document: &LabelDocument) -> LabelDocument {
    let mut saved = document.clone();
    for element in &mut saved.elements {
        match element {
            LabelElement::Image { bitmap, .. } => *bitmap = None,
            LabelElement::Object(LabelObject {
                content:
                    ObjectContent::Image {
                        bitmap, coverage, ..
                    },
                ..
            }) => {
                *bitmap = None;
                *coverage = None;
            }
            _ => {}
        }
    }
    saved
}

fn same_saved_element(current: &LabelElement, saved: &LabelElement) -> bool {
    match (current, saved) {
        (LabelElement::Object(current), LabelElement::Object(saved)) => {
            current.scene == saved.scene
                && current.sequence == saved.sequence
                && current.frame == saved.frame
                && current.layout == saved.layout
                && current.content_box == saved.content_box
                && current.border == saved.border
                && current.rotation == saved.rotation
                && current.flip_h == saved.flip_h
                && current.flip_v == saved.flip_v
                && same_saved_content(&current.content, &saved.content)
        }
        (
            LabelElement::Image {
                path,
                image_data,
                rotation,
                target_height,
                flip_h,
                flip_v,
                ..
            },
            LabelElement::Image {
                path: saved_path,
                image_data: saved_data,
                rotation: saved_rotation,
                target_height: saved_height,
                flip_h: saved_flip_h,
                flip_v: saved_flip_v,
                ..
            },
        ) => {
            path == saved_path
                && image_data == saved_data
                && rotation == saved_rotation
                && target_height == saved_height
                && flip_h == saved_flip_h
                && flip_v == saved_flip_v
        }
        _ => current == saved,
    }
}

fn same_saved_content(current: &ObjectContent, saved: &ObjectContent) -> bool {
    match (current, saved) {
        (
            ObjectContent::Image {
                path,
                image_data,
                preserve_alpha,
                ..
            },
            ObjectContent::Image {
                path: saved_path,
                image_data: saved_data,
                preserve_alpha: saved_alpha,
                ..
            },
        ) => path == saved_path && image_data == saved_data && preserve_alpha == saved_alpha,
        _ => current == saved,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |file| file.write_all(bytes))
}

/// Keep replacement as the final fallible operation. An error before that
/// point leaves the old file intact and drops the temporary file.
fn write_atomic_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    if path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Select a file name",
        ));
    }
    // Follow an existing file link, rather than replacing the link itself.
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)?,
        Ok(_) => path.to_path_buf(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let existing_permissions = match fs::metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "The target is not a file",
                ));
            }
            let permissions = metadata.permissions();
            if permissions.readonly() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "The file is read-only",
                ));
            }
            Some(permissions)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".ptouch-save-")
        .tempfile_in(parent)?;
    write(temporary.as_file_mut())?;
    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::bitmap::LabelBitmap;
    use ptouch_render::document::TextBox;
    use ptouch_render::text::TextAlign;

    fn document(content: &str) -> LabelDocument {
        LabelDocument {
            version: 4,
            tape_width_mm: 12,
            dpi: 180,
            font_name: "DejaVuSans".into(),
            font_margin: 0,
            flip_h: false,
            flip_v: false,
            elements: vec![LabelElement::Object(LabelObject::text(
                content,
                TextBox::default(),
            ))],
        }
    }

    #[test]
    fn edits_and_undo_compare_saved_content_instead_of_revisions() {
        let saved = document("PORT 01");
        let session = DocumentSession::new(&saved);
        assert!(!session.is_modified(&saved));
        let mut edited = document("PORT 02");
        assert!(session.is_modified(&edited));
        edited.elements = saved.elements.clone();
        assert!(!session.is_modified(&edited));
        edited.dpi = 360;
        assert!(session.is_modified(&edited));
        edited.dpi = saved.dpi;
        edited.elements[0].as_object_mut().unwrap().frame.x_mm += 0.1;
        assert!(session.is_modified(&edited));
        assert!(!session.is_modified(&saved));
    }

    #[test]
    fn empty_layout_settings_can_be_modified_and_saved() {
        let directory = tempfile::tempdir().unwrap();
        let mut empty = document("");
        empty.elements.clear();
        let mut session = DocumentSession::new(&empty);
        assert!(!session.is_modified(&empty));
        empty.tape_width_mm = 6;
        assert!(session.is_modified(&empty));
        let path = directory.path().join("empty.ptl");
        session.save(SaveKind::SaveAs, Some(&path), &empty).unwrap();
        assert!(!session.is_modified(&empty));
        let restored = LabelDocument::from_toml_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(restored, empty);
    }

    #[test]
    fn open_and_reset_update_file_identity_and_clean_baseline() {
        let first = document("FIRST");
        let second = document("SECOND");
        let mut session = DocumentSession::new(&first);
        session.opened("opened file.ptl".into(), &second);
        assert_eq!(session.current_path(), Some(Path::new("opened file.ptl")));
        assert!(!session.is_modified(&second));
        assert!(session.is_modified(&first));
        session.reset(&first);
        assert_eq!(session.current_path(), None);
        assert!(!session.is_modified(&first));
    }

    #[test]
    fn decoded_image_caches_do_not_mark_the_document_modified() {
        let mut saved = document("PORT 01");
        saved.elements.push(LabelElement::Image {
            path: None,
            image_data: vec![1, 2, 3],
            bitmap: None,
            rotation: 0.0,
            target_height: None,
            flip_h: false,
            flip_v: false,
        });
        saved
            .elements
            .push(LabelElement::Object(LabelObject::image_from_bytes(
                None,
                vec![4, 5, 6],
                TextBox::default(),
            )));
        let session = DocumentSession::new(&saved);
        let mut rendered = saved.clone();
        if let LabelElement::Image { bitmap, .. } = &mut rendered.elements[1] {
            *bitmap = Some(LabelBitmap::new(20, 10));
        }
        if let ObjectContent::Image {
            bitmap, coverage, ..
        } = &mut rendered.elements[2].as_object_mut().unwrap().content
        {
            *bitmap = Some(LabelBitmap::new(20, 10));
            *coverage = Some(Box::new(LabelBitmap::new(20, 10)));
        }
        assert!(!session.is_modified(&rendered));
        if let ObjectContent::Image { image_data, .. } =
            &mut rendered.elements[2].as_object_mut().unwrap().content
        {
            image_data.push(7);
        }
        assert!(session.is_modified(&rendered));
    }

    #[test]
    fn save_and_save_as_choose_the_correct_target() {
        let original = document("PORT 01");
        let mut session = DocumentSession::new(&original);
        assert!(session.needs_save_path(SaveKind::Save));
        assert_eq!(session.save_target(SaveKind::Save, None), None);
        assert_eq!(
            session.save_target(SaveKind::Save, Some(Path::new("new label"))),
            Some(PathBuf::from("new label.ptl"))
        );
        session.opened("existing label".into(), &original);
        assert!(!session.needs_save_path(SaveKind::Save));
        assert!(session.needs_save_path(SaveKind::SaveAs));
        assert_eq!(
            session.save_target(SaveKind::Save, Some(Path::new("ignored.ptl"))),
            Some(PathBuf::from("existing label"))
        );
        assert_eq!(session.save_target(SaveKind::SaveAs, None), None);
        assert_eq!(
            session.save_target(SaveKind::SaveAs, Some(Path::new("copy.txt"))),
            Some(PathBuf::from("copy.txt"))
        );
    }

    #[test]
    fn unicode_and_space_paths_save_replace_and_change_identity() {
        let directory = tempfile::tempdir().unwrap();
        let selected = directory.path().join("配线架 label");
        let path = selected.with_extension("ptl");
        let original = document("PORT 01");
        let mut session = DocumentSession::new(&original);
        assert_eq!(
            session
                .save(SaveKind::SaveAs, Some(&selected), &original)
                .unwrap(),
            Some(path.clone())
        );
        assert_eq!(session.current_path(), Some(path.as_path()));
        let changed = document("PORT 02");
        assert!(session.is_modified(&changed));
        assert_eq!(
            session.save(SaveKind::Save, None, &changed).unwrap(),
            Some(path.clone())
        );
        let restored = LabelDocument::from_toml_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(restored, changed);
        assert!(!session.is_modified(&changed));
        let copy = directory.path().join("second label.ptl");
        session
            .save(SaveKind::SaveAs, Some(&copy), &changed)
            .unwrap();
        assert_eq!(session.current_path(), Some(copy.as_path()));
        assert_eq!(fs::read(path).unwrap(), fs::read(copy).unwrap());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn cancelled_save_as_keeps_current_path_and_modified_state() {
        let original = document("PORT 01");
        let changed = document("PORT 02");
        let mut session = DocumentSession::new(&original);
        session.opened("original.ptl".into(), &original);
        assert_eq!(
            session.save(SaveKind::SaveAs, None, &changed).unwrap(),
            None
        );
        assert_eq!(session.current_path(), Some(Path::new("original.ptl")));
        assert!(session.is_modified(&changed));
        assert!(!session.is_modified(&original));
    }

    #[test]
    fn failed_save_as_keeps_the_original_file_and_session() {
        let directory = tempfile::tempdir().unwrap();
        let original_path = directory.path().join("original.ptl");
        let original = document("PORT 01");
        let mut session = DocumentSession::new(&original);
        session
            .save(SaveKind::SaveAs, Some(&original_path), &original)
            .unwrap();
        let bytes = fs::read(&original_path).unwrap();
        let invalid_path = directory.path().join("missing").join("copy.ptl");
        let changed = document("PORT 02");
        assert!(
            session
                .save(SaveKind::SaveAs, Some(&invalid_path), &changed)
                .is_err()
        );
        assert_eq!(session.current_path(), Some(original_path.as_path()));
        assert!(session.is_modified(&changed));
        assert!(!session.is_modified(&original));
        assert_eq!(fs::read(original_path).unwrap(), bytes);
        assert!(!invalid_path.exists());
    }

    #[test]
    fn invalid_document_never_replaces_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.ptl");
        let original = document("PORT 01");
        let mut session = DocumentSession::new(&original);
        session
            .save(SaveKind::SaveAs, Some(&path), &original)
            .unwrap();
        let original_bytes = fs::read(&path).unwrap();
        let mut invalid = original.clone();
        invalid.elements[0].as_object_mut().unwrap().content_box =
            Some(ptouch_render::document::ContentBox {
                width_mm: -1.0,
                height_mm: 1.0,
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            });
        // A malformed rich-text span is rejected by the serializer.
        if let ObjectContent::Text { styles, .. } =
            &mut invalid.elements[0].as_object_mut().unwrap().content
        {
            styles.push(ptouch_render::styled_text::StyleSpan {
                start: 0,
                end: 1000,
                style: Default::default(),
            });
        }
        assert!(matches!(
            session.save(SaveKind::Save, None, &invalid),
            Err(SaveError::InvalidDocument(_))
        ));
        assert_eq!(fs::read(path).unwrap(), original_bytes);
        assert!(!session.is_modified(&original));
        assert!(session.is_modified(&invalid));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn directory_and_invalid_paths_are_rejected_without_changing_the_session() {
        let directory = tempfile::tempdir().unwrap();
        let original = document("PORT 01");
        let mut session = DocumentSession::new(&original);
        for path in [Path::new(""), Path::new("."), directory.path()] {
            assert!(
                session
                    .save(SaveKind::SaveAs, Some(path), &original)
                    .is_err()
            );
            assert_eq!(session.current_path(), None);
        }
        let invalid = directory.path().join("invalid\0name.ptl");
        assert!(
            session
                .save(SaveKind::SaveAs, Some(&invalid), &original)
                .is_err()
        );
        assert_eq!(session.current_path(), None);
        assert!(!session.is_modified(&original));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn read_only_targets_keep_their_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("read-only.ptl");
        fs::write(&path, b"original").unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut read_only = original_permissions.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&path, read_only).unwrap();
        let result = write_atomic(&path, b"replacement");
        let unchanged = fs::read(&path).unwrap();
        fs::set_permissions(&path, original_permissions).unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(unchanged, b"original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn partial_write_failure_keeps_old_contents_and_removes_the_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.ptl");
        fs::write(&path, b"complete original").unwrap();
        let result = write_atomic_with(&path, |file| {
            file.write_all(b"partial replacement")?;
            Err(io::Error::other("simulated write failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"complete original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_link_keeps_the_link_and_target_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("real label.ptl");
        let link = directory.path().join("linked label.ptl");
        fs::write(&target, b"original").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        write_atomic(&link, b"replacement").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&link).unwrap(), b"replacement");
        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn legacy_text_and_image_properties_are_part_of_the_saved_content() {
        let mut saved = document("PORT 01");
        saved.elements = vec![LabelElement::Text {
            content: "legacy".into(),
            font_size: None,
            text_box: None,
            align: TextAlign::Center,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
        }];
        let session = DocumentSession::new(&saved);
        let mut changed = saved.clone();
        if let LabelElement::Text { rotation, .. } = &mut changed.elements[0] {
            *rotation = 90.0;
        }
        assert!(session.is_modified(&changed));
        assert!(!session.is_modified(&saved));
    }
}
