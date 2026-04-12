# ADR-040: Vector Knowledge Store — LanceDB + fastembed

**Status:** Implemented  
**Date:** 2026-04-09  
**Plan:** 995

## Context

Spawned agents produce low-quality output because they lack context about past
decisions, learnings, and architectural patterns. The existing `knowledge_base`
table in convergio-kernel uses keyword-only search with no semantic understanding.

An agent tasked with "LanceDB-backed vector store" delivered a SQLite brute-force
implementation instead. Tests passed, CI was green, but the deliverable didn't
match the spec. No existing gate caught this.

## Decision

### 1. Vector Store: LanceDB (embedded, Rust native)
- Crate: `lancedb` on crates.io — embedded, no server, HNSW index
- Storage: `~/.convergio/data/knowledge.lance`
- Schema: id, content, source_type, source_id, org_id, agent_id, created_at, vector[384]
- O(log N) search via HNSW, not O(N) brute-force

### 2. Embeddings: fastembed (pure Rust, ONNX Runtime)
- Crate: `fastembed` — AllMiniLML6V2 model, 384 dimensions
- No Python, no subprocess, no MLX dependency
- Model downloads on first use (~25MB), cached thereafter

### 3. Write Paths
- Post-task-complete hook: auto-embeds task summaries
- Git commit sync: scheduled task embeds recent commits  
- MCP: `cvg_knowledge_write` for manual entries
- Baseline seed: ingests AGENTS.md, ADRs, plans, KB, CONSTITUTION

### 4. Read Paths
- Spawn-time injection: queries store, injects into TASK.md
- File context injection: reads referenced files into TASK.md
- MCP: `cvg_knowledge_search` for runtime queries
- Org-ask enrichment: vector context supplements grounded inference

### 5. SpecComplianceGate
- Blocks task submit if deliverable doesn't match spec
- Checks: required deps in Cargo.toml, required files exist
- Gate chain position: after TestGate, before PrCommitGate

## Consequences

- Agents start with semantic context from ~74 knowledge entries
- Quality enforcement: SpecComplianceGate prevents spec-mismatched submits
- Build requires `protoc` (LanceDB → Arrow → protobuf)
- First daemon start downloads embedding model (~25MB)
- async-in-sync bridge uses `std::thread::spawn` (not `block_in_place`)
