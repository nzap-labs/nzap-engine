//! The notebook library: the public collection (GitHub) and your own
//! notebooks (local files), with typed parameters, forking, export and
//! running on a runtime.

pub mod catalog;
pub mod params;
pub mod store;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub use catalog::{Catalog, CatalogEntry, CatalogStatus, DEFAULT_CATALOG_URL};
pub use params::{NotebookParam, ParamType};
pub use store::{LocalNotebook, NotebookDraft, NotebookPatch, NotebookStore};

use crate::error::{Error, Result};
use crate::session::{Emit, SessionManager};

const PUBLIC_PREFIX: &str = "public:";
const LOCAL_PREFIX: &str = "local:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
}

/// What the UI sees for either kind of notebook.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notebook {
    /// `public:<slug>` or `local:<id>`.
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    /// Absent from list responses until a single notebook is fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub params: Vec<NotebookParam>,
    pub visibility: Visibility,
    pub is_mine: bool,
    pub tags: Vec<String>,
    pub author: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub forked_from: Option<String>,
}

impl Notebook {
    fn public(entry: CatalogEntry, source: Option<String>) -> Self {
        Self {
            id: format!("{PUBLIC_PREFIX}{}", entry.slug),
            slug: entry.slug,
            title: entry.title,
            description: entry.description,
            source,
            params: entry.params,
            visibility: Visibility::Public,
            is_mine: false,
            tags: entry.tags,
            author: entry.author,
            created_at: None,
            updated_at: None,
            forked_from: None,
        }
    }

    fn local(notebook: LocalNotebook, with_source: bool) -> Self {
        Self {
            id: format!("{LOCAL_PREFIX}{}", notebook.id),
            slug: notebook.slug,
            title: notebook.title,
            description: notebook.description,
            source: with_source.then_some(notebook.source),
            params: notebook.params,
            visibility: Visibility::Private,
            is_mine: true,
            tags: Vec::new(),
            author: None,
            created_at: Some(notebook.created_at),
            updated_at: Some(notebook.updated_at),
            forked_from: notebook.forked_from,
        }
    }
}

/// The portable form of a notebook (`<slug>.nzap.json`), which is also the
/// shape a contribution to the public collection starts from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookFile {
    pub format: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: Vec<NotebookParam>,
    pub source: String,
}

pub const NOTEBOOK_FILE_FORMAT: &str = "nzap-notebook/1";

enum Id<'a> {
    Public(&'a str),
    Local(&'a str),
}

fn parse_id(id: &str) -> Result<Id<'_>> {
    if let Some(slug) = id.strip_prefix(PUBLIC_PREFIX) {
        Ok(Id::Public(slug))
    } else if let Some(local) = id.strip_prefix(LOCAL_PREFIX) {
        Ok(Id::Local(local))
    } else {
        Err(Error::not_found("Notebook not found."))
    }
}

fn read_only() -> Error {
    Error::invalid("Public notebooks are read-only. Fork it into your notebooks to change it.")
}

pub struct NotebookLibrary {
    catalog: Catalog,
    store: NotebookStore,
}

impl NotebookLibrary {
    pub fn new(catalog: Catalog, store: NotebookStore) -> Self {
        Self { catalog, store }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The public collection (by title) followed by yours.
    pub fn list(&self) -> Vec<Notebook> {
        let mut public: Vec<Notebook> =
            self.catalog.entries().into_iter().map(|entry| Notebook::public(entry, None)).collect();
        public.sort_by_key(|notebook| notebook.title.to_lowercase());
        public
            .extend(self.store.list().into_iter().map(|notebook| Notebook::local(notebook, false)));
        public
    }

    /// One notebook including its source (public sources are fetched and
    /// integrity-checked).
    pub async fn get(&self, id: &str) -> Result<Notebook> {
        match parse_id(id)? {
            Id::Public(slug) => {
                let entry = self
                    .catalog
                    .entry(slug)
                    .ok_or_else(|| Error::not_found("Notebook not found."))?;
                let source = self.catalog.source(slug).await?;
                Ok(Notebook::public(entry, Some(source)))
            }
            Id::Local(local) => Ok(Notebook::local(self.store.get(local)?, true)),
        }
    }

    pub fn create(&self, draft: NotebookDraft) -> Result<Notebook> {
        Ok(Notebook::local(self.store.create(draft)?, true))
    }

    pub fn update(&self, id: &str, patch: NotebookPatch) -> Result<Notebook> {
        match parse_id(id)? {
            Id::Public(_) => Err(read_only()),
            Id::Local(local) => Ok(Notebook::local(self.store.update(local, patch)?, true)),
        }
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        match parse_id(id)? {
            Id::Public(_) => Err(read_only()),
            Id::Local(local) => self.store.delete(local),
        }
    }

    /// Copy any notebook into your own collection.
    pub async fn fork(&self, id: &str) -> Result<Notebook> {
        let source = self.get(id).await?;
        let forked_from =
            matches!(source.visibility, Visibility::Public).then(|| source.slug.clone());
        let draft = NotebookDraft {
            slug: self.store.free_slug(&source.slug),
            title: source.title,
            description: source.description,
            source: source.source.unwrap_or_default(),
            params: source.params,
            forked_from,
        };
        self.create(draft)
    }

    /// `(filename, JSON)` for saving a notebook to disk.
    pub async fn export(&self, id: &str) -> Result<(String, String)> {
        let notebook = self.get(id).await?;
        let file = NotebookFile {
            format: NOTEBOOK_FILE_FORMAT.to_owned(),
            slug: notebook.slug.clone(),
            title: notebook.title,
            description: notebook.description,
            params: notebook.params,
            source: notebook.source.unwrap_or_default(),
        };
        Ok((format!("{}.nzap.json", notebook.slug), serde_json::to_string_pretty(&file)?))
    }

    /// Add a notebook from an exported `.nzap.json` file.
    pub fn import(&self, text: &str) -> Result<Notebook> {
        let file: NotebookFile = serde_json::from_str(text)
            .map_err(|_| Error::invalid("That is not an NZAP notebook file."))?;
        if file.format != NOTEBOOK_FILE_FORMAT {
            return Err(Error::invalid(format!("Unsupported notebook format '{}'.", file.format)));
        }
        let slug = self.store.free_slug(&file.slug);
        self.create(NotebookDraft {
            slug,
            title: file.title,
            description: file.description,
            source: file.source,
            params: file.params,
            forked_from: None,
        })
    }

    /// Run a notebook on a runtime: validate the values against its declared
    /// parameters, inject them as `params`, stream the output.
    pub async fn run(
        &self,
        manager: &SessionManager,
        id: &str,
        session: &str,
        values: &Map<String, Value>,
        emit: &Emit,
    ) -> Result<Value> {
        manager.get(session)?;
        let notebook = self.get(id).await?;
        let resolved = params::resolve(&notebook.params, values)?;
        let code =
            params::assemble_source(notebook.source.as_deref().unwrap_or_default(), &resolved);
        manager.history().log(
            session,
            "automation",
            json!({"op": "notebook", "notebook": notebook.id, "title": notebook.title}),
        );
        manager.execute(session, &code, Duration::from_secs(3600), true, emit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(dir: &std::path::Path) -> NotebookLibrary {
        NotebookLibrary::new(
            // An unreachable catalog URL: the bundled snapshot serves.
            Catalog::new(reqwest::Client::new(), "http://127.0.0.1:9/", dir.join("cache")),
            NotebookStore::new(dir.join("notebooks")),
        )
    }

    #[tokio::test]
    async fn public_notebooks_come_from_the_bundle_offline() {
        let dir = tempfile::tempdir().unwrap();
        let library = library(dir.path());
        let list = library.list();
        assert!(list.iter().all(|notebook| notebook.source.is_none()));
        let print = list.iter().find(|notebook| notebook.slug == "print-notebook").unwrap();
        assert_eq!(print.id, "public:print-notebook");
        assert!(!print.is_mine);

        let full = library.get("public:print-notebook").await.unwrap();
        assert!(full.source.unwrap().contains("print(params[\"string_to_print\"])"));
        assert!(library.delete("public:print-notebook").is_err());
        assert!(library.update("public:print-notebook", NotebookPatch::default()).is_err());
        assert!(library.get("nope").await.is_err());
    }

    #[tokio::test]
    async fn fork_export_import() {
        let dir = tempfile::tempdir().unwrap();
        let library = library(dir.path());
        let fork = library.fork("public:print-notebook").await.unwrap();
        assert!(fork.is_mine);
        assert_eq!(fork.forked_from.as_deref(), Some("print-notebook"));
        assert_eq!(fork.slug, "print-notebook");
        let second = library.fork("public:print-notebook").await.unwrap();
        assert_eq!(second.slug, "print-notebook-2");

        let (filename, text) = library.export(&fork.id).await.unwrap();
        assert_eq!(filename, "print-notebook.nzap.json");
        let imported = library.import(&text).unwrap();
        assert_eq!(imported.slug, "print-notebook-3");
        assert_eq!(imported.source, fork.source);
        assert!(library.import("{}").is_err());

        let mine: Vec<_> = library.list().into_iter().filter(|notebook| notebook.is_mine).collect();
        assert_eq!(mine.len(), 3);
    }
}
