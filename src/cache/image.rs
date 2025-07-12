use std::collections::HashMap;
use parking_lot::RwLock;
use crate::images::ImageConversionParams;

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

    pub fn get_image(&self, url: &str, params: &ImageConversionParams) -> Option<(Vec<u8>, String)> {
        let cache_key = self.generate_cache_key(url, params);
        let images = self.images.read();
        images.get(&cache_key).cloned()
    }

    pub fn store_image(&self, url: &str, params: &ImageConversionParams, data: Vec<u8>, content_type: String) {
        let cache_key = self.generate_cache_key(url, params);
        let mut images = self.images.write();
        images.insert(cache_key, (data, content_type));
    }
    
    fn generate_cache_key(&self, url: &str, params: &ImageConversionParams) -> String {
        format!("{}#{}", url, params.to_cache_key())
    }
} 