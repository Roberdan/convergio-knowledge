//! Extension trait implementation for convergio-knowledge.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use convergio_db::pool::ConnPool;
use convergio_types::events::{DomainEvent, EventFilter, EventKind};
use convergio_types::extension::{
    AppContext, Extension, Health, McpToolDef, Metric, Migration, ScheduledTask,
};
use convergio_types::manifest::{Capability, Manifest, ModuleKind};

use crate::routes::{knowledge_routes, KnowledgeState};
use crate::store::LanceVectorStore;

pub struct KnowledgeExtension {
    pool: ConnPool,
    lance_path: PathBuf,
    store: OnceLock<Option<Arc<LanceVectorStore>>>,
}

impl KnowledgeExtension {
    pub fn new(pool: ConnPool) -> Self {
        let lance_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".convergio/data/knowledge.lance");
        Self {
            pool,
            lance_path,
            store: OnceLock::new(),
        }
    }

    fn get_store(&self) -> Option<Arc<LanceVectorStore>> {
        self.store
            .get_or_init(|| {
                let path = self.lance_path.clone();
                // Spawn a separate thread to init LanceDB — avoids tokio runtime nesting issues
                let result = std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .ok()?;
                    rt.block_on(LanceVectorStore::open(&path)).ok()
                })
                .join()
                .ok()
                .flatten();
                match result {
                    Some(s) => Some(Arc::new(s)),
                    None => {
                        tracing::warn!("lance store unavailable — knowledge search disabled");
                        None
                    }
                }
            })
            .clone()
    }

    fn state(&self) -> Arc<KnowledgeState> {
        Arc::new(KnowledgeState {
            pool: self.pool.clone(),
            store: self.get_store(),
        })
    }
}

impl Extension for KnowledgeExtension {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "convergio-knowledge".to_string(),
            description: "Vector knowledge store — semantic memory for agents via embeddings"
                .to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            kind: ModuleKind::Platform,
            provides: vec![
                Capability {
                    name: "knowledge-vectors".to_string(),
                    version: "1.0.0".to_string(),
                    description: "Embedding-based semantic search over agent knowledge".to_string(),
                },
                Capability {
                    name: "knowledge-api".to_string(),
                    version: "1.0.0".to_string(),
                    description: "REST API for knowledge search and write".to_string(),
                },
            ],
            requires: vec![],
            agent_tools: vec![],
            required_roles: vec!["orchestrator".into(), "all".into()],
        }
    }

    fn migrations(&self) -> Vec<Migration> {
        vec![]
    }

    fn routes(&self, _ctx: &AppContext) -> Option<axum::Router> {
        Some(knowledge_routes(self.state()))
    }

    fn health(&self) -> Health {
        let Some(store) = self.get_store() else {
            return Health::Ok;
        };
        let ok = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            rt.block_on(store.stats()).ok()
        })
        .join()
        .ok()
        .flatten();
        if ok.is_some() {
            Health::Ok
        } else {
            Health::Degraded {
                reason: "lance stats failed".into(),
            }
        }
    }

    fn metrics(&self) -> Vec<Metric> {
        let Some(store) = self.get_store() else {
            return vec![];
        };
        let count = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            rt.block_on(store.stats())
                .ok()
                .map(|s| s.total_entries as f64)
        })
        .join()
        .ok()
        .flatten()
        .unwrap_or(0.0);
        vec![Metric {
            name: "knowledge_entries_total".to_string(),
            value: count,
            labels: vec![],
        }]
    }

    fn subscriptions(&self) -> Vec<EventFilter> {
        vec![EventFilter {
            kind_prefix: Some("Task".to_string()),
            org: None,
            actor: None,
        }]
    }

    fn on_event(&self, event: &DomainEvent) {
        if let EventKind::TaskCompleted { task_id } = &event.kind {
            if let Some(store) = self.get_store() {
                crate::hooks::on_task_completed(&self.pool, store, *task_id)
            };
        }
    }

    fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        vec![
            ScheduledTask {
                name: "knowledge-commit-sync",
                cron: "*/15 * * * *",
            },
            ScheduledTask {
                name: "knowledge-auto-prune",
                cron: "0 3 * * *", // daily at 3 AM
            },
        ]
    }

    fn on_scheduled_task(&self, task_name: &str) {
        if task_name == "knowledge-commit-sync" {
            if let Some(store) = self.get_store() {
                crate::hooks::sync_recent_commits(store)
            };
        }
        if task_name == "knowledge-auto-prune" {
            if let Some(store) = self.get_store() {
                let store = store.clone();
                tokio::spawn(async move {
                    let req = crate::pruning::PruneRequest {
                        max_age_days: None,
                        source_type: None,
                        dry_run: false,
                    };
                    let report = crate::pruning::prune(&store, &req).await;
                    tracing::info!(
                        stale = report.stale_removed,
                        dedup = report.duplicates_removed,
                        source_dedup = report.source_dedup_removed,
                        total = report.total_removed,
                        "auto-prune completed"
                    );
                });
            };
        }
    }

    fn mcp_tools(&self) -> Vec<McpToolDef> {
        crate::mcp_defs::knowledge_tools()
    }
}
