//! Go language parser implementation
//!
//! This module provides comprehensive Go language support for Codanna's code intelligence system,
//! enabling precise symbol extraction, relationship tracking, and semantic analysis of Go codebases.
//!
//! ## Overview
//!
//! The Go parser implementation uses Tree-sitter-go v0.23.4 to provide full support for modern Go
//! language features including generics (Go 1.18+), method receivers, embedded types, and Go's
//! unique package system with capitalization-based visibility.
//!
//! ## Key Features
//!
//! ### Symbol Extraction
//! - **Functions and Methods**: Complete signature extraction including receivers, parameters, and return types
//! - **Struct Types**: Field extraction, embedded struct detection, and method association
//! - **Interface Types**: Method signatures, embedded interface composition
//! - **Variables and Constants**: Package-level and function-scoped declarations
//! - **Type Aliases**: Full support for custom type definitions
//! - **Generic Types**: Type parameters and constraints (Go 1.18+)
//!
//! ### Go-Specific Language Features
//! - **Package System**: Import path resolution, module system integration
//! - **Visibility Rules**: Exported/unexported symbol detection via capitalization
//! - **Method Receivers**: Both value and pointer receiver methods
//! - **Interface Implementations**: Structural compatibility checking (implicit)
//! - **Embedded Types**: Struct and interface composition
//! - **Channel Operations**: Basic channel type recognition
//!
//! ### Performance Characteristics
//! - **Indexing Speed**: >10,000 symbols/second target
//! - **Memory Efficiency**: ~100 bytes per symbol
//! - **Resolution Speed**: <10ms semantic search operations
//!
//! ## Module Components
//!
//! - [`parser`]: Core Tree-sitter integration and symbol extraction
//! - [`behavior`]: Go-specific language behaviors and formatting rules
//! - [`definition`]: Language registration and Tree-sitter node mappings
//! - [`resolution`]: Symbol resolution, scope management, and type system integration
//!
//! ## Integration
//!
//! The Go parser integrates seamlessly with Codanna's MCP server, providing these tools:
//! - `find_symbol` / `search_symbols` - Locate Go symbols by name or pattern
//! - `get_calls` / `find_callers` - Navigate function call relationships
//! - `analyze_impact` - Assess change impact across Go packages
//! - `semantic_search_docs` - Natural language queries over Go code
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use codanna::parsing::go::{GoParser, GoBehavior};
//! use codanna::parsing::{LanguageParser, LanguageBehavior};
//!
//! // Create parser instance
//! let parser = GoParser::new();
//! let behavior = GoBehavior::new();
//!
//! // Parser handles all Go language constructs automatically
//! // through the unified LanguageParser interface
//! ```
//!
//! ## Documentation References
//!
//! For detailed implementation information, see:
//! - [`definition`] module for complete AST node mappings
//! - `contributing/parsers/go/NODE_MAPPING.md` for Tree-sitter node types
//! - `tests/fixtures/go/` for comprehensive code examples
//! - [`parser`] module for symbol extraction implementation details

pub mod audit;
pub mod behavior;
pub mod definition;
pub(crate) mod method_sets;
pub mod parser;
pub mod resolution;

pub use behavior::GoBehavior;
pub use definition::GoLanguage;
pub use parser::GoParser;
pub use resolution::{GoInheritanceResolver, GoResolutionContext};

// Re-export for registry registration
pub(crate) use definition::register;
