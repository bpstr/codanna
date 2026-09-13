//! Language detection and enumeration
//!
//! This module provides language detection from file extensions
//! and language-specific configuration.

use serde::{Deserialize, Serialize};

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Php,
    Go,
    C,
    Clojure,
    Cpp,
    CSharp,
    Gdscript,
    Java,
    Kotlin,
    Lua,
    Swift,
    Ruby,
    Bash,
}

impl Language {
    /// Convert to LanguageId for registry usage
    pub fn to_language_id(&self) -> super::LanguageId {
        match self {
            Language::Rust => super::LanguageId::new("rust"),
            Language::Python => super::LanguageId::new("python"),
            Language::JavaScript => super::LanguageId::new("javascript"),
            Language::TypeScript => super::LanguageId::new("typescript"),
            Language::Php => super::LanguageId::new("php"),
            Language::Go => super::LanguageId::new("go"),
            Language::C => super::LanguageId::new("c"),
            Language::Clojure => super::LanguageId::new("clojure"),
            Language::Cpp => super::LanguageId::new("cpp"),
            Language::CSharp => super::LanguageId::new("csharp"),
            Language::Gdscript => super::LanguageId::new("gdscript"),
            Language::Java => super::LanguageId::new("java"),
            Language::Kotlin => super::LanguageId::new("kotlin"),
            Language::Lua => super::LanguageId::new("lua"),
            Language::Swift => super::LanguageId::new("swift"),
            Language::Ruby => super::LanguageId::new("ruby"),
            Language::Bash => super::LanguageId::new("bash"),
        }
    }

    /// Create Language from LanguageId (for backward compatibility)
    pub fn from_language_id(id: super::LanguageId) -> Option<Self> {
        match id.as_str() {
            "rust" => Some(Language::Rust),
            "python" => Some(Language::Python),
            "javascript" => Some(Language::JavaScript),
            "typescript" => Some(Language::TypeScript),
            "php" => Some(Language::Php),
            "go" => Some(Language::Go),
            "c" => Some(Language::C),
            "clojure" => Some(Language::Clojure),
            "cpp" => Some(Language::Cpp),
            "csharp" => Some(Language::CSharp),
            "gdscript" => Some(Language::Gdscript),
            "java" => Some(Language::Java),
            "kotlin" => Some(Language::Kotlin),
            "lua" => Some(Language::Lua),
            "swift" => Some(Language::Swift),
            "ruby" => Some(Language::Ruby),
            "bash" => Some(Language::Bash),
            _ => None,
        }
    }

    /// Detect language from file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext_lower = ext.to_lowercase();
        let registry = super::get_registry();
        if let Ok(registry) = registry.lock() {
            if let Some(def) = registry.get_by_extension(&ext_lower) {
                return Self::from_language_id(def.id());
            }
        }

        match ext_lower.as_str() {
            "rs" => Some(Language::Rust),
            "py" | "pyi" => Some(Language::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "php" | "php3" | "php4" | "php5" | "php7" | "php8" | "phps" | "phtml" => {
                Some(Language::Php)
            }
            "go" | "go.mod" | "go.sum" => Some(Language::Go),
            "c" | "h" => Some(Language::C),
            "clj" | "cljs" | "cljc" | "edn" => Some(Language::Clojure),
            "cpp" | "hpp" | "cc" | "cxx" | "hxx" => Some(Language::Cpp),
            "cs" | "csx" => Some(Language::CSharp),
            "gd" => Some(Language::Gdscript),
            "java" => Some(Language::Java),
            "kt" | "kts" => Some(Language::Kotlin),
            "lua" => Some(Language::Lua),
            "swift" => Some(Language::Swift),
            "rb" | "rake" | "gemspec" => Some(Language::Ruby),
            "sh" | "bash" | "zsh" | "bats" => Some(Language::Bash),
            _ => None,
        }
    }

    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    pub fn extensions(&self) -> &[&str] {
        match self {
            Language::Rust => &["rs"],
            Language::Python => &["py", "pyi"],
            Language::JavaScript => &["js", "jsx", "mjs", "cjs"],
            Language::TypeScript => &["ts", "tsx", "mts", "cts"],
            Language::Php => &[
                "php", "php3", "php4", "php5", "php7", "php8", "phps", "phtml",
            ],
            Language::Go => &["go", "go.mod", "go.sum"],
            Language::C => &["c", "h"],
            Language::Clojure => &["clj", "cljs", "cljc", "edn"],
            Language::Cpp => &["cpp", "hpp", "cc", "cxx", "hxx"],
            Language::CSharp => &["cs", "csx"],
            Language::Gdscript => &["gd"],
            Language::Java => &["java"],
            Language::Kotlin => &["kt", "kts"],
            Language::Lua => &["lua"],
            Language::Swift => &["swift"],
            Language::Ruby => &["rb", "rake", "gemspec"],
            Language::Bash => &["sh", "bash", "zsh", "bats"],
        }
    }

    pub fn config_key(&self) -> &str {
        match self {
            Language::Rust => "rust",
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Php => "php",
            Language::Go => "go",
            Language::C => "c",
            Language::Clojure => "clojure",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Gdscript => "gdscript",
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::Lua => "lua",
            Language::Swift => "swift",
            Language::Ruby => "ruby",
            Language::Bash => "bash",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Language::Rust => "Rust",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Php => "PHP",
            Language::Go => "Go",
            Language::C => "C",
            Language::Clojure => "Clojure",
            Language::Cpp => "C++",
            Language::CSharp => "C#",
            Language::Gdscript => "GDScript",
            Language::Java => "Java",
            Language::Kotlin => "Kotlin",
            Language::Lua => "Lua",
            Language::Swift => "Swift",
            Language::Ruby => "Ruby",
            Language::Bash => "Bash / POSIX shell",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension("RS"), Some(Language::Rust));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("pyi"), Some(Language::Python));
        assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("jsx"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("php"), Some(Language::Php));
        assert_eq!(Language::from_extension("PHP"), Some(Language::Php));
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("gd"), Some(Language::Gdscript));
        assert_eq!(Language::from_extension("lua"), Some(Language::Lua));
        assert_eq!(Language::from_extension("rb"), Some(Language::Ruby));
        assert_eq!(Language::from_extension("RB"), Some(Language::Ruby));
        assert_eq!(Language::from_extension("sh"), Some(Language::Bash));
        assert_eq!(Language::from_extension("bats"), Some(Language::Bash));
        assert_eq!(Language::from_extension("txt"), None);
    }

    #[test]
    fn test_language_from_path() {
        assert_eq!(
            Language::from_path(Path::new("main.rs")),
            Some(Language::Rust)
        );
        assert_eq!(
            Language::from_path(Path::new("script.py")),
            Some(Language::Python)
        );
        assert_eq!(
            Language::from_path(Path::new("app.js")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            Language::from_path(Path::new("types.d.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            Language::from_path(Path::new("index.php")),
            Some(Language::Php)
        );
        assert_eq!(
            Language::from_path(Path::new("main.go")),
            Some(Language::Go)
        );
        assert_eq!(
            Language::from_path(Path::new("main.cpp")),
            Some(Language::Cpp)
        );
        assert_eq!(
            Language::from_path(Path::new("player.gd")),
            Some(Language::Gdscript)
        );
        assert_eq!(
            Language::from_path(Path::new("script.lua")),
            Some(Language::Lua)
        );
        assert_eq!(
            Language::from_path(Path::new("app/models/user.rb")),
            Some(Language::Ruby)
        );
        assert_eq!(
            Language::from_path(Path::new("scripts/deploy.sh")),
            Some(Language::Bash)
        );
        assert_eq!(Language::from_path(Path::new("README.md")), None);
    }

    #[test]
    fn test_extensions() {
        assert!(Language::Rust.extensions().contains(&"rs"));
        assert!(Language::Python.extensions().contains(&"py"));
        assert!(Language::JavaScript.extensions().contains(&"js"));
        assert!(Language::TypeScript.extensions().contains(&"ts"));
        assert!(Language::Php.extensions().contains(&"php"));
        assert!(Language::Go.extensions().contains(&"go"));
        assert!(Language::Gdscript.extensions().contains(&"gd"));
        assert!(Language::Lua.extensions().contains(&"lua"));
        assert!(Language::Ruby.extensions().contains(&"rb"));
        assert!(Language::Bash.extensions().contains(&"sh"));
    }
}
