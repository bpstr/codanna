#!/usr/bin/env python3
"""Apply reviewed dimension guards to pinned sources. No remote writes."""
import hashlib
from pathlib import Path

EXPECTED = {
    'src/semantic/mod.rs': '5b9d11743943ca3ae60d8941d15013083e6729b9',
    'src/semantic/journal.rs': '1a7dcf8d26ea4b175551bb7b5a02371ee63e9bc0',
    'src/indexing/facade.rs': '4f1404b646bda385e4525f26b0744edcf2342740',
    'src/main.rs': 'db735afc4c9f93e0621bcd2cf69795e47ce159d3',
    'src/rebuild_plan.rs': '105d246f2ce14f277404b9bd2209b498f082a57d',
    'src/indexing/pipeline/stages/semantic_embed.rs': 'b3a175f7d38245df4f072bf4d673169ce448cce2',
}

def git_hash(data):
    return hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()

contents = {}
for name, expected in EXPECTED.items():
    data = Path(name).read_bytes()
    if git_hash(data) != expected:
        raise SystemExit(f'Unreviewed source drift: {name}')
    contents[name] = data.decode('utf-8')


def replace(name, before, after):
    if contents[name].count(before) != 1:
        raise SystemExit(f'Expected one patch site: {name}: {before[:80]}')
    contents[name] = contents[name].replace(before, after)

new_module = '''//! The code journal's dimension contract, distinct from document backends.
use super::{EmbeddingBackend, SemanticSearchError};
use crate::config::SemanticSearchConfig;
use crate::{IndexError, IndexResult};

/// Maximum readable code-journal dimension. Does not limit document vectors.
pub const MAX_CODE_EMBEDDING_DIMENSION: usize = 4096;

/// Validate before code inference and before publishing any semantic artifact.
pub fn validate_code_embedding_dimension(dimension: usize) -> Result<(), SemanticSearchError> {
    if (1..=MAX_CODE_EMBEDDING_DIMENSION).contains(&dimension) {
        return Ok(());
    }
    Err(SemanticSearchError::StorageError {
        message: format!(
            "Code embedding dimension {dimension} is unsupported; expected 1..={MAX_CODE_EMBEDDING_DIMENSION}"
        ),
        suggestion: "Choose a supported code embedding dimension. Preserve existing index files; changing the cache limit does not change the journal format.".into(),
    })
}

/// Resolve an explicit remote code dimension without initializing a provider.
/// An environment override takes precedence over the file, exactly as at runtime.
pub(crate) fn configured_code_dimension(
    config: &SemanticSearchConfig,
) -> Result<Option<usize>, SemanticSearchError> {
    if std::env::var("CODANNA_EMBED_URL").is_err() && config.remote_url.is_none() {
        return Ok(None);
    }
    let dimension = match std::env::var("CODANNA_EMBED_DIM") {
        Ok(value) => Some(value.parse::<usize>().map_err(|_| {
            SemanticSearchError::ModelInitError(format!(
                "CODANNA_EMBED_DIM must be an integer in 1..={MAX_CODE_EMBEDDING_DIMENSION}"
            ))
        })?),
        Err(std::env::VarError::NotPresent) => config.remote_dim,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(SemanticSearchError::ModelInitError(
                "CODANNA_EMBED_DIM must be a Unicode integer".into(),
            ));
        }
    };
    if let Some(dimension) = dimension {
        validate_code_embedding_dimension(dimension)?;
    }
    Ok(dimension)
}

/// Code-only wrapper. Shared document construction retains its own contract.
/// Unknown remote dimensions require a probe, but never source inference here.
pub fn build_code_embedding_backend(config: &SemanticSearchConfig) -> IndexResult<EmbeddingBackend> {
    configured_code_dimension(config).map_err(IndexError::SemanticSearch)?;
    let backend = crate::indexing::facade::build_embedding_backend(config)?;
    validate_code_embedding_dimension(backend.dimensions()).map_err(IndexError::SemanticSearch)?;
    Ok(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_dimension_contract_accepts_only_readable_journal_dimensions() {
        for dimension in [1, 2, 4096] {
            validate_code_embedding_dimension(dimension).unwrap();
        }
        for dimension in [0, 4097, 16384, usize::MAX] {
            let error = validate_code_embedding_dimension(dimension).unwrap_err().to_string();
            assert!(error.contains("1..=4096"));
            assert!(error.contains(&dimension.to_string()));
        }
    }
}
'''
new_path = 'src/semantic/code_dimension.rs'
if Path(new_path).exists():
    raise SystemExit(f'Unexpected existing module: {new_path}')
replace('src/semantic/mod.rs', 'mod journal;\n', 'mod code_dimension;\nmod journal;\n')
replace('src/semantic/mod.rs', 'pub use metadata::{', '''pub use code_dimension::{
    MAX_CODE_EMBEDDING_DIMENSION, build_code_embedding_backend, validate_code_embedding_dimension,
};
pub(crate) use code_dimension::configured_code_dimension;
pub use metadata::{''')
replace('src/semantic/journal.rs', '''    if !(1..=4).contains(&value.metadata.version) || !(1..=4096).contains(&value.metadata.dimension)
    {
        return Err(error("unsupported semantic metadata version or dimension"));
    }
''', '''    if !(1..=4).contains(&value.metadata.version) {
        return Err(error("unsupported semantic metadata version"));
    }
    super::validate_code_embedding_dimension(value.metadata.dimension)?;
''')
replace('src/semantic/journal.rs', '''    mut metadata: SemanticMetadata,
) -> Result<(), SemanticSearchError> {
    fs::create_dir_all(path).map_err(error)?;
''', '''    mut metadata: SemanticMetadata,
) -> Result<(), SemanticSearchError> {
    // Reject even the first/empty checkpoint before creating a directory, lock,
    // generation file or cache. Reader and writer use the same code contract.
    super::validate_code_embedding_dimension(metadata.dimension)?;
    fs::create_dir_all(path).map_err(error)?;
''')
replace('src/indexing/facade.rs', '''    pub fn enable_semantic_search(&mut self) -> FacadeResult<()> {
        let semantic_path = self.index_base.join("semantic");
        std::fs::create_dir_all(&semantic_path)?;

        let backend = build_embedding_backend(&self.settings.semantic_search)?;
        let backend = Arc::new(backend);
''', '''    pub fn enable_semantic_search(&mut self) -> FacadeResult<()> {
        let backend = crate::semantic::build_code_embedding_backend(&self.settings.semantic_search)?;
        self.enable_semantic_search_with_backend(backend)
    }

    fn enable_semantic_search_with_backend(&mut self, backend: EmbeddingBackend) -> FacadeResult<()> {
        crate::semantic::validate_code_embedding_dimension(backend.dimensions())?;
        let semantic_path = self.index_base.join("semantic");
        std::fs::create_dir_all(&semantic_path)?;
        let backend = Arc::new(backend);
''')
replace('src/indexing/facade.rs', '''    pub fn ensure_embedding_pool(&mut self) -> FacadeResult<()> {
        self.check_semantic_state()?;
        let backend = match self.embedding_pool.get() {
            Some(backend) => Arc::clone(backend),
            None => Arc::new(build_embedding_backend(&self.settings.semantic_search)?),
        };
        if let Some(semantic) = &self.semantic_search {
''', '''    pub fn ensure_embedding_pool(&mut self) -> FacadeResult<()> {
        self.check_semantic_state()?;
        let backend = match self.embedding_pool.get() {
            Some(backend) => Arc::clone(backend),
            None => Arc::new(crate::semantic::build_code_embedding_backend(&self.settings.semantic_search)?),
        };
        self.bind_embedding_backend(backend)
    }

    /// Reuse CLI preflight's backend rather than repeating a possibly paid probe.
    /// A loaded generation still has to match dimensions and the complete identity.
    pub fn install_prepared_code_backend(&mut self, backend: EmbeddingBackend) -> FacadeResult<()> {
        self.check_semantic_state()?;
        if self.semantic_search.is_none() {
            self.enable_semantic_search_with_backend(backend)
        } else {
            self.bind_embedding_backend(Arc::new(backend))
        }
    }

    fn bind_embedding_backend(&mut self, backend: Arc<EmbeddingBackend>) -> FacadeResult<()> {
        crate::semantic::validate_code_embedding_dimension(backend.dimensions())?;
        if let Some(semantic) = &self.semantic_search {
''')
replace('src/indexing/facade.rs', '''        let _ = self.embedding_pool.set(backend);
        tracing::debug!("Initialized embedding backend on first semantic operation");
''', '''        let _ = self.embedding_pool.take();
        let _ = self.embedding_pool.set(backend);
        tracing::debug!("Initialized embedding backend on first semantic operation");
''')
replace('src/main.rs', '''    let mut indexer: Option<IndexFacade> = if !needs_indexer {
''', '''    // Validate the code backend before a force/emission-heal lane can clear
    // an existing index. An unknown dimension may need one probe, not source
    // inference. Reuse this exact backend after facade construction.
    let mut prepared_index_backend = if matches!(cli.command, Commands::Index { .. })
        && config.semantic_search.enabled
    {
        match codanna::semantic::build_code_embedding_backend(&config.semantic_search) {
            Ok(backend) => Some(backend),
            Err(error) => {
                eprintln!("Error: code embedding preflight failed: {error}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };
    let mut indexer: Option<IndexFacade> = if !needs_indexer {
''')
replace('src/main.rs', '''        // Only enable semantic search for commands that need it
        if needs_semantic_search
''', '''        // The indexing lane already validated before destructive setup. Loaded
        // vectors must still match this backend; do not silently fall back to a
        // successful lexical-only rebuild when semantic initialization failed.
        if let Some(backend) = prepared_index_backend.take() {
            if let Err(error) = idx.install_prepared_code_backend(backend) {
                eprintln!("Error: prepared code embedding backend is incompatible: {error}");
                std::process::exit(1);
            }
            eprintln!("{}", format_semantic_status(&config.semantic_search));
        }
        // Only enable semantic search for commands that need it
        if needs_semantic_search
''')
replace('src/rebuild_plan.rs', '''    let dimension = match std::env::var("CODANNA_EMBED_DIM") {
        Ok(value) => Some(
            value
                .parse::<usize>()
                .map_err(|_| failure("CODANNA_EMBED_DIM must be a positive integer"))?,
        ),
        Err(_) => cfg.remote_dim,
    };
''', '''    let dimension = crate::semantic::configured_code_dimension(cfg)
        .map_err(IndexError::SemanticSearch)?;
''')
replace('src/rebuild_plan.rs', '''    if dimension == 0 {
        return Err(failure(
            "Remote embedding dimension must be greater than zero",
        ));
    }
''', '')
replace('src/indexing/pipeline/stages/semantic_embed.rs', '''    pub(crate) fn process_batch(&self, batch: &EmbeddingBatch) -> PipelineResult<usize> {
        let accelerated = crate::memory::accelerated_embeddings_requested();
''', '''    pub(crate) fn process_batch(&self, batch: &EmbeddingBatch) -> PipelineResult<usize> {
        crate::semantic::validate_code_embedding_dimension(self.pool.dimensions())
            .map_err(|error| PipelineError::Parse {
                path: Default::default(),
                reason: error.to_string(),
            })?;
        let accelerated = crate::memory::accelerated_embeddings_requested();
''')
for name, content in contents.items():
    Path(name).write_text(content, encoding='utf-8')
Path(new_path).write_text(new_module, encoding='utf-8')
print('Applied code-only dimension contract to pinned source files.')
