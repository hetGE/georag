//! Everything a command or screen needs, opened once: settings, registry,
//! store and the resident engine.

use std::sync::Arc;

use georag_engine::InferenceEngine;
use georag_store::Store;

use crate::chat::ChatContext;
use crate::config::Settings;
use crate::engine::{self, LoadPlan};
use crate::error::{CoreError, Result};
use crate::home::Home;
use crate::models::{ModelRole, Registry, catalog};

pub struct App {
    pub home: Home,
    pub settings: Settings,
    pub registry: Registry,
    pub store: Arc<Store>,
    pub engine: Arc<dyn InferenceEngine>,
    pub plan: LoadPlan,
    pub doc_prefix: String,
    pub query_prefix: String,
}

impl App {
    /// Open the store and load the in-use models. `want_n_ctx` is a manual
    /// tier's context, `None` for the largest rung that fits.
    pub fn open(home: &Home, want_n_ctx: Option<u32>) -> Result<App> {
        if !home.is_initialised() {
            home.init()?;
        }
        let settings = Settings::load(&home.settings_path())?;
        let registry = Registry::load(home)?;
        let store = Arc::new(Store::open(&home.db_path())?);
        let resident = engine::start(home, &settings, &registry, want_n_ctx)?;
        let (doc_prefix, query_prefix) = match registry.in_use(ModelRole::Embed) {
            Some(m) => {
                let dim = m.dim.unwrap_or(0) as usize;
                if dim > 0 {
                    match store.ensure_vectors(&m.id, dim) {
                        Ok(()) => {}
                        Err(georag_store::StoreError::DimensionMismatch {
                            existing,
                            requested,
                        }) => {
                            return Err(CoreError::Invalid(format!(
                                "the index was built with a {existing}-dimension embedding model and {} produces {requested}: re-embed with `georag index --reprocess` after confirming, or switch back with `georag models use embed ID`",
                                m.id
                            )));
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                let (d, q) = catalog::embed_prefixes_for(&m.id).unwrap_or(("", ""));
                (d.to_string(), q.to_string())
            }
            None => (String::new(), String::new()),
        };
        Ok(App {
            home: home.clone(),
            settings,
            registry,
            store,
            engine: resident.engine,
            plan: resident.plan,
            doc_prefix,
            query_prefix,
        })
    }

    pub fn search(
        &self,
        query: &str,
        tags: &[String],
        k: usize,
    ) -> Result<Vec<crate::retrieval::SearchResult>> {
        crate::retrieval::search(
            &self.store,
            self.engine.as_ref(),
            query,
            tags,
            k,
            &self.query_prefix,
        )
    }

    /// Seed the synthetic demo corpus; returns total chat tokens seeded.
    pub fn seed_demo(&self, sites: usize, chunks_per_doc: usize) -> Result<u32> {
        crate::devseed::seed(
            &self.store,
            self.engine.as_ref(),
            sites,
            chunks_per_doc,
            &self.doc_prefix,
        )
    }

    /// One chunk with its file path, for the sources pane.
    pub fn chunk(&self, chunk_id: i64) -> Result<Option<georag_store::Chunk>> {
        Ok(self.store.chunks_by_ids(&[chunk_id])?.into_iter().next())
    }

    /// Absolute path of a file: its root's path joined with the relative path.
    pub fn file_path(&self, file_id: i64) -> Result<Option<std::path::PathBuf>> {
        let Some(file) = self.store.file_get(file_id)? else {
            return Ok(None);
        };
        let roots = self.store.roots()?;
        Ok(roots
            .into_iter()
            .find(|r| r.id == file.root_id)
            .map(|r| std::path::Path::new(&r.path).join(&file.rel_path)))
    }

    pub fn chat_context(&self) -> ChatContext<'_> {
        ChatContext {
            store: &self.store,
            engine: self.engine.as_ref(),
            settings: &self.settings,
            query_prefix: self.query_prefix.clone(),
            max_ctx: self.plan.n_ctx.max(512),
        }
    }
}
