// Focused compatibility regressions for defects tracked in bartolli/codanna.
//
// These tests protect fork behavior that overlaps an open or recently fixed
// upstream issue. Keep the fixtures minimal so an upstream implementation can
// later be compared against the fork without pulling in fork-specific product
// architecture.

#[path = "upstream/named_import_alias.rs"]
mod named_import_alias;

#[path = "upstream/unresolved_import.rs"]
mod unresolved_import;

#[path = "upstream/watcher_config_reload.rs"]
mod watcher_config_reload;

#[path = "upstream/relationship_cache_scale.rs"]
mod relationship_cache_scale;
