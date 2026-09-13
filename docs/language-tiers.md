# Language support tiers

Codanna uses two parsing tiers so broad language coverage does not make its common paths slower or less trustworthy.

## Rich parsers

Codanna's native parsers remain the first choice for languages with dedicated support. They provide language-specific symbols plus higher-confidence relationships such as calls, implementations, inheritance, receiver/type resolution, imports, and module behavior. The Ruby/Bash extension brings this tier to 17 language families in this fork.

A file matching a rich parser never enters the generic language pack.

## Generic syntax tier

For every registry miss, Codanna can fall back to `tree-sitter-language-pack` 1.16.2. That release exposes 371 grammar-backed language names and aliases while sharing Codanna's tree-sitter 0.26 ABI.

The generic tier deliberately limits itself to portable syntax facts:

- language detection from file paths/extensions;
- functions, methods, classes, structs, interfaces, enums, modules, namespaces and traits where the grammar pack exposes them;
- variable/constant/type symbols where available;
- signatures and supported documentation comments;
- import paths and aliases/wildcards where available;
- normal Codanna indexing, lexical lookup, semantic embeddings, document/knowledge integration and incremental change detection after extraction.

The generic tier does **not** claim deep semantic relationships. It does not guess calls, inheritance, implementations, receiver types, runtime-generated symbols, macros, reflection or metaprogramming when the generic grammar layer cannot establish them. Those capabilities remain a reason to promote important languages to rich parsers over time.

## Performance model

Language detection is metadata-only. Walking a repository does not load or download grammar binaries.

Rich languages take the existing Codanna parser path with no generic-pack work. Generic parsers are resolved during parser preflight, before destructive incremental cleanup. A parser cache miss can download that language's prebuilt grammar to the language-pack cache; subsequent indexing uses the persistent local cache. Repositories therefore pay parser setup only for generic languages they actually contain, not for the whole 371-language catalogue.

Generic source processing has a 16 MiB per-file ceiling and a 5 second parse timeout. These limits fail the file rather than silently truncating it.

For fully offline environments, warm the needed language-pack cache before disconnecting. A future CLI surface can expose explicit bulk prewarming; indexing already performs the same parser-resolution step automatically during preflight.

## Configuration

Generic languages are enabled by default when their path is recognized by the pack. An existing `languages.<name>` settings entry can explicitly disable one. Rich-language enable/disable behavior remains unchanged and takes precedence over generic fallback.

## Provenance and licensing

The broad tier is built on `tree-sitter-language-pack` rather than copying Codebase Memory's C indexing engine. The architecture is inspired by the same useful separation visible in codebase-memory-mcp: very broad tree-sitter syntax coverage with deeper semantics for a smaller set of important languages.

`tree-sitter-language-pack` is MIT licensed and aggregates upstream tree-sitter grammars with their own permissive licenses. The dependency is version-pinned so grammar ABI changes cannot silently alter Codanna builds.
