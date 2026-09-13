//! Import statement representation
//!
//! This module defines the Import struct used by language parsers
//! to represent import statements extracted from source files.

use crate::FileId;

/// Represents an import statement in a file
#[derive(Debug, Clone)]
pub struct Import {
    /// The path being imported (e.g., "std::collections::HashMap")
    pub path: String,
    /// The symbol name exported by the target module when it differs from
    /// the local binding (TypeScript/JavaScript: `Foo as LocalFoo`).
    pub imported_name: Option<String>,
    /// The alias if any (e.g., "use foo::Bar as Baz")
    pub alias: Option<String>,
    /// Location in the file where this import appears
    pub file_id: FileId,
    /// Whether this is a glob import (e.g., "use foo::*")
    pub is_glob: bool,
    /// Whether this is a type-only import (TypeScript: `import type { Foo }`)
    pub is_type_only: bool,
}
