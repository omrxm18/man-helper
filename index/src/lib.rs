mod cache;
mod finder;
mod indexer;
mod search;

pub use cache::cache_path;
pub use finder::{discover_pages, PageRef};
pub use indexer::{build_index, FlagEntry};
pub use search::search;

/// Load the cached index if present, otherwise return None so the
/// caller can decide whether to build one (it's an explicit choice,
/// not automatic, since building can take a while on a big system).
pub fn load_cached() -> Option<Vec<FlagEntry>> {
    cache::load()
}

pub fn save_cache(entries: &[FlagEntry]) -> std::io::Result<()> {
    cache::save(entries)
}