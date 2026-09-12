# Ruby and Bash language support

This extension adds native tree-sitter indexing for Ruby and Bash/POSIX-style shell files while preserving Codanna's local, low-overhead indexing model.

## Ruby

Recognized extensions: `.rb`, `.rake`, `.gemspec`.

Indexed structure:

- classes and modules
- instance and singleton methods
- method/function calls with caller context
- `require`, `require_relative`, and `load` dependency imports
- class inheritance
- class/module → method definition relationships
- contiguous `#` documentation comments

The parser does not execute Ruby, Bundler, Rails, Rake, or application code. Dynamic metaprogramming such as `define_method`, runtime `send`, autoload conventions, Rails DSL-generated methods, and monkey patches may not produce complete static relationships.

## Bash / shell

Recognized extensions: `.sh`, `.bash`, `.zsh`, `.bats`.

Indexed structure:

- function definitions
- command calls with function caller context
- `source` / `.` file dependencies
- contiguous `#` documentation comments

The parser never executes shell source. Dynamic commands constructed through `eval`, variable expansion, aliases, functions loaded from unknown runtime paths, shell-specific plugins, or generated source cannot be resolved statically.

## Performance model

Both languages share a compact parser implementation and use the same single-pass tree traversal primitives. There is no language server, interpreter process, package-manager invocation, network request, or model call during parsing. Tree-sitter grammar state is kept in the parser instance and normal Codanna incremental indexing applies unchanged.

Relationship extraction deliberately favors high-confidence structural edges. Missing dynamic relationships are preferable to guessed edges that would pollute impact analysis and agent context.
