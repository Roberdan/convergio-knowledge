//! MCP tool definitions for the knowledge extension.

use convergio_types::extension::McpToolDef;
use serde_json::json;

pub fn knowledge_tools() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            name: "cvg_knowledge_search".into(),
            description: "Search the vector knowledge store with semantic similarity.".into(),
            method: "POST".into(),
            path: "/api/knowledge/search".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query text"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default 5)"
                    },
                    "org_id": {
                        "type": "string",
                        "description": "Filter by organization"
                    },
                    "source_type": {
                        "type": "string",
                        "description": "Filter by source: task, commit, doc, agent_memory, kb"
                    },
                    "project_id": {
                        "type": "string",
                        "description": "Filter by project/repo (e.g. 'convergio', 'istitutodeimpresa')"
                    }
                },
                "required": ["query"]
            }),
            min_ring: "sandboxed".into(),
            path_params: vec![],
        },
        McpToolDef {
            name: "cvg_knowledge_write".into(),
            description: "Write an entry to the vector knowledge store.".into(),
            method: "POST".into(),
            path: "/api/knowledge/write".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Text content to embed and store"
                    },
                    "source_type": {
                        "type": "string",
                        "description": "Source type: task, commit, doc, agent_memory, kb"
                    },
                    "source_id": {
                        "type": "string",
                        "description": "Source identifier (task_id, commit hash, etc.)"
                    },
                    "org_id": {
                        "type": "string",
                        "description": "Organization scope"
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Agent that produced this knowledge"
                    },
                    "project_id": {
                        "type": "string",
                        "description": "Project/repo scope (e.g. 'convergio')"
                    }
                },
                "required": ["content", "source_type", "source_id"]
            }),
            min_ring: "trusted".into(),
            path_params: vec![],
        },
    ]
}
