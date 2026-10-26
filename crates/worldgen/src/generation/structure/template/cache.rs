//! Template caching for embedded structure templates.
//!
//! This module provides a lazy-loading cache for structure templates that are
//! embedded in the binary as generated static definitions.

use std::sync::Arc;

use dashmap::DashMap;

use super::StructureTemplate;

/// Vanilla's implicit namespace.
const DEFAULT_NAMESPACE: &str = "minecraft";

/// Canonicalizes a resource id to fully-qualified `namespace:path` form.
///
/// A bare `foo` becomes `minecraft:foo`, matching vanilla resolution.
fn canonicalize(name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("{DEFAULT_NAMESPACE}:{name}")
    }
}

/// A cache for loaded structure templates.
///
/// Templates are loaded lazily on first access and stored for reuse.
/// Keys are fully-qualified resource ids, so `foo` and `minecraft:foo`
/// share a single entry.
/// The cache is thread-safe and can be accessed from multiple threads.
pub struct TemplateCache {
    cache: DashMap<String, Arc<StructureTemplate>>,
}

impl Default for TemplateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateCache {
    /// Creates a new empty template cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    /// Gets a template by `name`, loading it from embedded resources if not cached.
    ///
    /// `name` may be bare (`foo`) or namespaced (`minecraft:foo`, `pumpkin:foo`).
    ///
    /// Returns the loaded template wrapped in an `Arc`, or `None` if the template
    /// doesn't exist in the embedded definitions.
    pub fn get(&self, name: &str) -> Option<Arc<StructureTemplate>> {
        let key = canonicalize(name);

        // Check cache first
        if let Some(template) = self.cache.get(&key) {
            return Some(Arc::clone(&template));
        }

        let static_template = pumpkin_data::structure_template::get_structure_template(&key)?;
        let mut template = StructureTemplate::from_static(static_template);
        template.name = Some(key.clone());

        let arc = Arc::new(template);
        self.cache.insert(key, Arc::clone(&arc));
        Some(arc)
    }

    /// Preloads a list of templates into the cache.
    ///
    /// This can be useful during server startup to avoid loading delays
    /// during gameplay.
    pub fn preload(&self, names: &[&str]) {
        for name in names {
            if self.get(name).is_none() {
                tracing::warn!("Unknown embedded structure template: {name}");
            }
        }
    }

    /// Returns the number of cached templates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Clears all cached templates.
    pub fn clear(&self) {
        self.cache.clear();
    }
}

/// Global template cache instance.
///
/// This provides a singleton cache that can be used throughout the codebase
/// without needing to pass around a cache reference.
static GLOBAL_CACHE: std::sync::LazyLock<TemplateCache> =
    std::sync::LazyLock::new(TemplateCache::new);

/// Gets the global template cache.
#[must_use]
pub fn global_cache() -> &'static TemplateCache {
    &GLOBAL_CACHE
}

/// Gets a template by `name` from the global cache.
///
/// Returns the loaded template wrapped in an `Arc`, or `None` if not found.
#[must_use]
pub fn get_template(name: &str) -> Option<Arc<StructureTemplate>> {
    global_cache().get(name)
}

/// Returns a list of all available template names that can be loaded.
///
/// These are derived from the embedded structure files at compile time.
/// Names are fully qualified (e.g. `minecraft:village/plains/houses/...`).
/// Useful for tab-completion in commands.
#[must_use]
pub const fn all_template_names() -> &'static [&'static str] {
    pumpkin_data::structure_template::all_template_names()
}

/// Returns a list of all available structure names for `/place structure` tab-completion.
#[must_use]
pub const fn all_structure_names() -> &'static [&'static str] {
    pumpkin_data::structures::StructureKeys::all_names()
}

/// Returns a list of all available pool names for `/place jigsaw` tab-completion.
#[must_use]
pub const fn all_pool_names() -> &'static [&'static str] {
    pumpkin_data::template_pool::StaticTemplatePool::all_names()
}

#[must_use]
pub const fn all_embedded_datapack_names() -> &'static [&'static str] {
    pumpkin_data::structure_template::all_embedded_datapack_names()
}
