use std::collections::HashMap;
use parking_lot::RwLock;

// Image cache for proxied images - stored in memory forever
pub struct ImageCache {
    images: RwLock<HashMap<String, (Vec<u8>, String)>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            images: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_image(&self, url: &str) -> Option<(Vec<u8>, String)> {
        let images = self.images.read();
        images.get(url).cloned()
    }

    pub fn store_image(&self, url: &str, data: Vec<u8>, content_type: String) {
        let mut images = self.images.write();
        images.insert(url.to_string(), (data, content_type));
    }
} 