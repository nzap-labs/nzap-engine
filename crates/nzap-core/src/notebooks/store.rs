//! Your own notebooks, stored locally (one JSON file each) — the desktop
//! replacement for hosted NZAP's private `notebooks` rows.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::params::{self, NotebookParam};
use crate::error::{Error, Result};
use crate::secrets::write_private;

pub const MAX_DESCRIPTION_CHARS: usize = 2000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalNotebook {
    pub id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub source: String,
    #[serde(default)]
    pub params: Vec<NotebookParam>,
    pub created_at: String,
    pub updated_at: String,
    /// The public slug this notebook was forked from.
    #[serde(default)]
    pub forked_from: Option<String>,
    /// Its app spec (`nzap-app/1`), when it is an app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<Value>,
}

/// A new notebook, or a full replacement from the editor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookDraft {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub source: String,
    #[serde(default)]
    pub params: Vec<NotebookParam>,
    #[serde(default)]
    pub forked_from: Option<String>,
    #[serde(default)]
    pub app: Option<Value>,
}

/// A partial update; absent fields are left alone.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookPatch {
    pub slug: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub params: Option<Vec<NotebookParam>>,
    /// A new app spec; `null` turns the notebook back into a plain one.
    #[serde(default, deserialize_with = "present")]
    pub app: Option<Value>,
}

/// Keep an explicit `null` as `Some(Value::Null)`, so it differs from an
/// absent field.
fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// Lowercase letters, digits and dashes (hosted NZAP's rule).
pub fn valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    (2..=63).contains(&slug.len())
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn check(draft: &NotebookDraft) -> Result<()> {
    if !valid_slug(draft.slug.trim()) {
        return Err(Error::invalid("Slug must be lowercase letters, numbers and dashes."));
    }
    if draft.title.trim().is_empty() {
        return Err(Error::invalid("A title is required."));
    }
    if draft.source.trim().is_empty() {
        return Err(Error::invalid("Notebook source is required."));
    }
    if draft.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(Error::invalid("The description is longer than 2000 characters."));
    }
    if draft.app.as_ref().is_some_and(|app| !super::supported_app(app)) {
        return Err(Error::invalid(format!(
            "The app spec must be a {} object of at most 64 KB.",
            super::APP_FORMAT
        )));
    }
    params::validate(&draft.params)
}

pub struct NotebookStore {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl NotebookStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, lock: Mutex::new(()) }
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if valid_id(id) {
            Ok(self.dir.join(format!("{id}.json")))
        } else {
            Err(Error::not_found("Notebook not found."))
        }
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Every stored notebook, by title. Unreadable files are skipped.
    pub fn list(&self) -> Vec<LocalNotebook> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut notebooks: Vec<LocalNotebook> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .filter_map(|text| serde_json::from_str::<LocalNotebook>(&text).ok())
            .filter(|notebook| valid_id(&notebook.id))
            .collect();
        notebooks.sort_by_key(|notebook| notebook.title.to_lowercase());
        notebooks
    }

    pub fn get(&self, id: &str) -> Result<LocalNotebook> {
        let text = std::fs::read_to_string(self.path(id)?)
            .map_err(|_| Error::not_found("Notebook not found."))?;
        serde_json::from_str(&text).map_err(|_| Error::internal("That notebook file is corrupt."))
    }

    fn slug_taken(&self, slug: &str, except: Option<&str>) -> bool {
        self.list()
            .iter()
            .any(|notebook| notebook.slug == slug && Some(notebook.id.as_str()) != except)
    }

    fn save(&self, notebook: &LocalNotebook) -> Result<()> {
        write_private(&self.path(&notebook.id)?, &serde_json::to_vec_pretty(notebook)?)
    }

    pub fn create(&self, draft: NotebookDraft) -> Result<LocalNotebook> {
        check(&draft)?;
        let _guard = self.guard();
        let slug = draft.slug.trim().to_owned();
        if self.slug_taken(&slug, None) {
            return Err(Error::invalid(format!("You already have a notebook called '{slug}'.")));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let notebook = LocalNotebook {
            id: uuid::Uuid::new_v4().simple().to_string(),
            slug,
            title: draft.title.trim().to_owned(),
            description: draft.description,
            source: draft.source,
            params: draft.params,
            created_at: now.clone(),
            updated_at: now,
            forked_from: draft.forked_from,
            app: draft.app,
        };
        self.save(&notebook)?;
        Ok(notebook)
    }

    pub fn update(&self, id: &str, patch: NotebookPatch) -> Result<LocalNotebook> {
        let _guard = self.guard();
        let mut notebook = self.get(id)?;
        let mut draft = NotebookDraft {
            slug: patch.slug.unwrap_or_else(|| notebook.slug.clone()),
            title: patch.title.unwrap_or_else(|| notebook.title.clone()),
            description: patch.description.unwrap_or_else(|| notebook.description.clone()),
            source: patch.source.unwrap_or_else(|| notebook.source.clone()),
            params: patch.params.unwrap_or_else(|| notebook.params.clone()),
            forked_from: notebook.forked_from.clone(),
            app: match patch.app {
                Some(Value::Null) => None,
                Some(app) => Some(app),
                None => notebook.app.clone(),
            },
        };
        draft.slug = draft.slug.trim().to_owned();
        check(&draft)?;
        if self.slug_taken(&draft.slug, Some(id)) {
            return Err(Error::invalid(format!(
                "You already have a notebook called '{}'.",
                draft.slug
            )));
        }
        notebook.slug = draft.slug;
        notebook.title = draft.title.trim().to_owned();
        notebook.description = draft.description;
        notebook.source = draft.source;
        notebook.params = draft.params;
        notebook.app = draft.app;
        notebook.updated_at = chrono::Utc::now().to_rfc3339();
        self.save(&notebook)?;
        Ok(notebook)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _guard = self.guard();
        std::fs::remove_file(self.path(id)?).map_err(|_| Error::not_found("Notebook not found."))
    }

    /// A slug not yet used locally, based on `base` (`base`, `base-2`, …).
    pub fn free_slug(&self, base: &str) -> String {
        let taken: std::collections::HashSet<String> =
            self.list().into_iter().map(|n| n.slug).collect();
        if !taken.contains(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| {
                let suffix = format!("-{n}");
                let stem: String = base.chars().take(63 - suffix.len()).collect();
                format!("{stem}{suffix}")
            })
            .find(|candidate| !taken.contains(candidate))
            .unwrap_or_else(|| base.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft(slug: &str) -> NotebookDraft {
        NotebookDraft {
            slug: slug.into(),
            title: format!("Title {slug}"),
            description: String::new(),
            source: "print(params['x'])".into(),
            params: serde_json::from_value(json!([{"key": "x", "label": "X", "type": "string"}]))
                .unwrap(),
            forked_from: None,
            app: None,
        }
    }

    #[test]
    fn crud_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = NotebookStore::new(dir.path().join("notebooks"));
        assert!(store.list().is_empty());

        let created = store.create(draft("mine")).unwrap();
        assert_eq!(created.id.len(), 32);
        assert_eq!(store.get(&created.id).unwrap(), created);
        assert!(store.create(draft("mine")).is_err(), "slugs are unique");

        let updated = store
            .update(
                &created.id,
                NotebookPatch { title: Some("Renamed".into()), ..NotebookPatch::default() },
            )
            .unwrap();
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.source, created.source);
        assert!(store
            .update(
                &created.id,
                NotebookPatch { source: Some("  ".into()), ..NotebookPatch::default() }
            )
            .is_err());

        let other = store.create(draft("other")).unwrap();
        assert!(store
            .update(
                &other.id,
                NotebookPatch { slug: Some("mine".into()), ..NotebookPatch::default() }
            )
            .is_err());
        assert_eq!(store.list().len(), 2);

        store.delete(&created.id).unwrap();
        assert!(store.get(&created.id).is_err());
        assert!(store.delete(&created.id).is_err());
        assert!(store.get("../../secrets").is_err(), "ids never become paths");
    }

    #[test]
    fn a_null_app_in_a_patch_differs_from_no_app() {
        let clear: NotebookPatch = serde_json::from_value(json!({"app": null})).unwrap();
        assert_eq!(clear.app, Some(Value::Null));
        let keep: NotebookPatch = serde_json::from_value(json!({"title": "x"})).unwrap();
        assert_eq!(keep.app, None);
    }

    #[test]
    fn validation() {
        let dir = tempfile::tempdir().unwrap();
        let store = NotebookStore::new(dir.path().to_path_buf());
        for bad in [
            NotebookDraft { slug: "Bad Slug".into(), ..draft("x") },
            NotebookDraft { title: " ".into(), ..draft("ok-1") },
            NotebookDraft { source: "".into(), ..draft("ok-2") },
            NotebookDraft { description: "x".repeat(2001), ..draft("ok-3") },
            NotebookDraft { app: Some(json!({"format": "nzap-app/9"})), ..draft("ok-4") },
            NotebookDraft { app: Some(json!(["not", "an", "object"])), ..draft("ok-5") },
        ] {
            assert!(store.create(bad).is_err());
        }
    }

    #[test]
    fn free_slugs() {
        let dir = tempfile::tempdir().unwrap();
        let store = NotebookStore::new(dir.path().to_path_buf());
        assert_eq!(store.free_slug("print-notebook"), "print-notebook");
        store.create(draft("print-notebook")).unwrap();
        assert_eq!(store.free_slug("print-notebook"), "print-notebook-2");
        store.create(draft("print-notebook-2")).unwrap();
        assert_eq!(store.free_slug("print-notebook"), "print-notebook-3");
    }
}
