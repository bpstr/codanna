//! Configuration module for the codebase intelligence system.
//!
//! This module provides a layered configuration system that supports:
//! - Default values
//! - TOML configuration file
//! - Environment variable overrides
//! - CLI argument overrides
//!
//! # Environment Variables
//!
//! Environment variables must be prefixed with `CI_` and use double underscores
//! to separate nested levels:
//! - `CI_INDEXING__PARALLELISM=8` sets `indexing.parallelism`
//! - `CI_LOGGING__DEFAULT=debug` sets `logging.default`
//! - `CI_INDEXING__INCLUDE_TESTS=false` sets `indexing.include_tests`
//!
//! For logging, use `RUST_LOG` environment variable directly (standard Rust pattern).

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

mod defaults;
mod init;
mod paths;

use defaults::*;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Settings {
    /// Version of the configuration schema
    #[serde(default = "default_version")]
    pub version: u32,

    /// Path to the index directory
    #[serde(default = "default_index_path")]
    pub index_path: PathBuf,

    /// Workspace root directory (where .codanna is located)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,

    /// Indexing configuration
    #[serde(default)]
    pub indexing: IndexingConfig,

    /// Cached canonicalized paths for fast lookups (not serialized)
    #[serde(skip)]
    pub indexed_paths_cache: Vec<PathBuf>,

    /// Language-specific settings (IndexMap preserves insertion order)
    #[serde(default)]
    pub languages: IndexMap<String, LanguageConfig>,

    /// MCP server settings
    #[serde(default)]
    pub mcp: McpConfig,

    /// Semantic search settings
    #[serde(default)]
    pub semantic_search: SemanticSearchConfig,

    /// File watching settings
    #[serde(default)]
    pub file_watch: FileWatchConfig,

    /// Server settings (stdio/http mode)
    #[serde(default)]
    pub server: ServerConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// AI guidance settings for multi-hop queries
    #[serde(default)]
    pub guidance: GuidanceConfig,

    /// Document embedding settings for RAG
    #[serde(default)]
    pub documents: crate::documents::DocumentsConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct IndexingConfig {
    /// CPU cores to use for indexing (0 = auto-detect all cores)
    /// Thread counts for each stage are derived from this value
    #[serde(default = "default_parallelism")]
    pub parallelism: usize,

    /// Tantivy heap size in megabytes
    /// Controls memory usage before flushing to disk
    #[serde(default = "default_tantivy_heap_mb")]
    pub tantivy_heap_mb: usize,

    /// Maximum retry attempts for transient file system errors
    /// Handles permission delays from antivirus, SELinux, etc.
    #[serde(default = "default_max_retry_attempts")]
    pub max_retry_attempts: u32,

    /// Maximum source file size read into memory. Files larger than this
    /// are rejected before allocation. Defaults to 32 MiB.
    #[serde(default = "default_max_file_size_bytes")]
    pub max_file_size_bytes: u64,

    /// List of directories to index
    /// This list is managed by the add-dir and remove-dir commands
    #[serde(default)]
    pub indexed_paths: Vec<PathBuf>,

    // Pipeline settings (parallel indexer)
    /// Symbols per batch before flushing to Tantivy
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    /// Batches to accumulate before Tantivy commit
    #[serde(default = "default_batches_per_commit")]
    pub batches_per_commit: usize,

    /// Enable detailed pipeline stage tracing (timing, memory, throughput)
    /// Set logging.modules.pipeline = "info" to see output
    #[serde(default)]
    pub pipeline_tracing: bool,

    /// Show progress bars during indexing (default: true)
    #[serde(default = "default_true")]
    pub show_progress: bool,
}

/// Source layout for project resolution
/// Determines how source roots are discovered from build configuration files
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SourceLayout {
    /// Standard JVM layout: src/main/{lang}, src/test/{lang}
    #[default]
    Jvm,
    /// Standard Kotlin Multiplatform: src/commonMain/kotlin, src/jvmMain/kotlin, etc.
    StandardKmp,
    /// Flat KMP layout (ktor-style): common/src/, jvm/src/, posix/src/
    FlatKmp,
}

/// Per-project configuration with explicit source layout
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProjectConfig {
    /// Path to the project configuration file (e.g., build.gradle.kts)
    pub config_file: PathBuf,

    /// Source layout for this project
    #[serde(default)]
    pub source_layout: SourceLayout,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LanguageConfig {
    /// Whether this language is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// File extensions for this language
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Additional parser options
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub parser_options: HashMap<String, serde_json::Value>,

    /// Project configuration files to monitor (e.g., tsconfig.json, pyproject.toml)
    /// Empty by default - project resolution is opt-in
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_files: Vec<PathBuf>,

    /// Per-project configuration with explicit source layout
    /// Use when auto-detection fails (e.g., custom build plugins)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpConfig {
    /// Maximum context size in bytes
    #[serde(default = "default_max_context_size")]
    pub max_context_size: usize,

    /// `Host` allowlist for Streamable HTTP inbound. None ⇒ loopback-only default.
    #[serde(default)]
    pub allowed_hosts: Option<Vec<String>>,

    /// `Origin` allowlist for Streamable HTTP inbound. None ⇒ no Origin check.
    #[serde(default)]
    pub allowed_origins: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SemanticSearchConfig {
    /// Enable semantic search
    #[serde(default = "default_false")]
    pub enabled: bool,

    /// Model to use for embeddings
    #[serde(default = "default_embedding_model")]
    pub model: String,

    /// Similarity threshold for search results
    #[serde(default = "default_similarity_threshold")]
    pub threshold: f32,

    /// Number of parallel embedding model instances
    #[serde(default = "default_embedding_threads")]
    pub embedding_threads: usize,

    /// Remote embedding server URL (OpenAI-compatible, e.g. http://host:8100).
    /// When set, local fastembed is bypassed and this endpoint is used instead.
    /// Overrideable via CODANNA_EMBED_URL env var.
    #[serde(default)]
    pub remote_url: Option<String>,

    /// Model name to send to the remote embedding server.
    /// Overrideable via CODANNA_EMBED_MODEL env var.
    #[serde(default)]
    pub remote_model: Option<String>,

    /// Output dimension of the remote embedding model.
    /// Required when remote_url is set. Overrideable via CODANNA_EMBED_DIM env var.
    #[serde(default)]
    pub remote_dim: Option<usize>,
    // API key: set CODANNA_EMBED_API_KEY environment variable.
    // Intentionally not a config field -- secrets must not live in shared config files.
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FileWatchConfig {
    /// Enable automatic file watching for indexed files
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Debounce interval in milliseconds (default: 500ms)
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerConfig {
    /// Default server mode: "stdio" or "http"
    #[serde(default = "default_server_mode")]
    pub mode: String,

    /// HTTP server bind address
    #[serde(default = "default_bind_address")]
    pub bind: String,

    /// Watch interval for stdio mode (seconds)
    #[serde(default = "default_watch_interval")]
    pub watch_interval: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    /// Default log level for all modules
    /// Valid values: "error", "warn", "info", "debug", "trace"
    #[serde(default = "default_log_level")]
    pub default: String,

    /// Per-module log level overrides (IndexMap preserves insertion order)
    /// Example: { "tantivy" = "warn", "watcher" = "debug" }
    #[serde(default)]
    pub modules: IndexMap<String, String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            default: default_log_level(),
            modules: default_logging_modules(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GuidanceConfig {
    /// Enable AI guidance system
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Templates for specific tools
    #[serde(default)]
    pub templates: IndexMap<String, GuidanceTemplate>,

    /// Global template variables
    #[serde(default)]
    pub variables: IndexMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GuidanceTemplate {
    /// Template for no results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_results: Option<String>,

    /// Template for single result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_result: Option<String>,

    /// Template for multiple results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_results: Option<String>,

    /// Custom templates for specific count ranges
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<GuidanceRange>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GuidanceRange {
    /// Minimum count (inclusive)
    pub min: usize,
    /// Maximum count (inclusive, None = unbounded)
    pub max: Option<usize>,
    /// Template to use
    pub template: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: default_version(),
            index_path: default_index_path(),
            workspace_root: None,
            indexing: IndexingConfig::default(),
            indexed_paths_cache: Vec::new(),
            languages: generate_language_defaults(),
            mcp: McpConfig::default(),
            semantic_search: SemanticSearchConfig::default(),
            file_watch: FileWatchConfig::default(),
            server: ServerConfig::default(),
            logging: LoggingConfig::default(),
            guidance: GuidanceConfig::default(),
            documents: crate::documents::DocumentsConfig::default(),
        }
    }
}

impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            parallelism: default_parallelism(),
            tantivy_heap_mb: default_tantivy_heap_mb(),
            max_retry_attempts: default_max_retry_attempts(),
            max_file_size_bytes: default_max_file_size_bytes(),
            indexed_paths: Vec::new(),
            batch_size: default_batch_size(),
            batches_per_commit: default_batches_per_commit(),
            pipeline_tracing: false,
            show_progress: true,
        }
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            max_context_size: default_max_context_size(),
            allowed_hosts: None,
            allowed_origins: None,
        }
    }
}

impl Default for SemanticSearchConfig {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            model: default_embedding_model(),
            threshold: default_similarity_threshold(),
            embedding_threads: default_embedding_threads(),
            remote_url: None,
            remote_model: None,
            remote_dim: None,
        }
    }
}

impl Default for FileWatchConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            debounce_ms: default_debounce_ms(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            mode: default_server_mode(),
            bind: default_bind_address(),
            watch_interval: default_watch_interval(),
        }
    }
}

impl Default for GuidanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            templates: default_guidance_templates(),
            variables: default_guidance_variables(),
        }
    }
}

impl Settings {
    pub fn load() -> Result<Self, figment::Error> {
        let local_dir = crate::init::local_dir_name();
        let config_path = PathBuf::from(local_dir).join("settings.toml");
        let mut figment = Figment::from(Serialized::defaults(Settings::default()));
        if config_path.exists() {
            figment = figment.merge(Toml::file(config_path));
        }
        figment.merge(Env::prefixed("CI_").split("__")).extract()
    }
}
