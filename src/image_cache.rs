//! A bounded image cache for `img()` elements.
//!
//! GPUI's default keeps every decoded image (and its GPU texture) for the
//! life of the app, and `RetainAllImageCache` only frees on drop. A chat
//! client shows an unbounded stream of photos and avatars, so this evicts
//! the least recently used images once their decoded size passes a budget,
//! removing their textures from the window too.

use std::{collections::HashMap, sync::Arc};

use gpui_kit::{
    App, AppContext, Entity, ImageCache, ImageCacheError, ImageCacheItem, RenderImage, Resource,
    Window, hash,
};

/// Decoded bytes kept before evicting: a screenful of photos and avatars
/// with room to scroll back; older ones reload from disk.
const BUDGET: usize = 96 * 1024 * 1024;

struct Entry {
    item: ImageCacheItem,
    /// Decoded size, once loaded.
    bytes: usize,
    last_used: u64,
}

pub struct LruImageCache {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    clock: u64,
}

impl LruImageCache {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
        });
        cx.observe_release(&cache, |cache, cx| {
            for (_, entry) in cache.entries.drain() {
                if let Some(Ok(image)) = entry.item.get() {
                    cx.drop_image(image, None);
                }
            }
        })
        .detach();
        cache
    }

    /// Drops every image and its texture (the window was hidden).
    pub fn clear(&mut self, cx: &mut App) {
        for (_, entry) in self.entries.drain() {
            if let Some(Ok(image)) = entry.item.get() {
                cx.drop_image(image, None);
            }
        }
        self.bytes = 0;
    }

    fn evict(&mut self, keep: u64, window: &mut Window, cx: &mut App) {
        while self.bytes > BUDGET {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(key, entry)| **key != keep && entry.bytes > 0)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes -= entry.bytes;
                if let Some(Ok(image)) = entry.item.get() {
                    cx.drop_image(image, Some(window));
                }
            }
        }
    }
}

fn decoded_bytes(image: &RenderImage) -> usize {
    let size = image.size(0);
    let frame = (size.width.0.max(0) as usize) * (size.height.0.max(0) as usize) * 4;
    frame * image.frame_count().max(1)
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);
        self.clock += 1;
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            item: ImageCacheItem::new(resource, cx),
            bytes: 0,
            last_used: 0,
        });
        entry.last_used = self.clock;
        let result = entry.item.use_image(window);
        // Account for the image the first time it is seen loaded.
        if entry.bytes == 0
            && let Some(Ok(image)) = &result
        {
            entry.bytes = decoded_bytes(image).max(1);
            self.bytes += entry.bytes;
            self.evict(key, window, cx);
        }
        result
    }
}
