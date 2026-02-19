//! Two-phase memory pipeline for rustcode.
//!
//! Memories give the agent persistent knowledge across sessions.
//!
//! # Pipeline
//!
//! **Phase 1 — Extraction** (runs after session completion):
//! Reads the last 50 user/assistant messages and calls the LLM to extract
//! key facts, preferences, and decisions as bullet points.
//! Saved to `~/.config/rustcode/memories/raw/{session_id}.md`.
//!
//! **Phase 2 — Consolidation** (runs on background scheduler every 10 min):
//! Reads all raw memory files and synthesizes them into a single summary.
//! Saved to `~/.config/rustcode/memories/summary.md`.
//!
//! The consolidated summary is injected into the system prompt at session start.
//!
//! # Usage
//!
//! ```rust,ignore
//! use rustcode_memories::storage::MemoryStorage;
//! use rustcode_memories::phase1::extract_memories;
//! use rustcode_memories::phase2::consolidate_memories;
//! use rustcode_memories::prompts::build_memory_section;
//!
//! // At session start: inject existing summary
//! if let Some(storage) = MemoryStorage::new() {
//!     if let Some(summary) = storage.load_summary() {
//!         let section = build_memory_section(&summary.content);
//!         // prepend to system prompt
//!     }
//! }
//!
//! // After session completes: extract memories in background
//! tokio::spawn(async move {
//!     let _ = extract_memories(&session_id, &messages, llm, &storage, &model).await;
//! });
//! ```

pub mod model;
pub mod phase1;
pub mod phase2;
pub mod prompts;
pub mod storage;

pub use model::{MemorySummary, RawMemory};
pub use prompts::build_memory_section;
pub use storage::{memories_dir, MemoryStorage};
