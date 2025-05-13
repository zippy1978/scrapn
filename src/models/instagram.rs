use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use log;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramPost {
    pub id: String,
    pub shortcode: String,
    pub display_url: String,
    pub thumbnail_url: Option<String>,
    pub caption: Option<String>,
    pub likes_count: Option<u64>,
    pub comments_count: Option<u64>,
    pub timestamp: Option<DateTime<Utc>>,
    pub is_video: bool,
    pub video_url: Option<String>,
    pub video_view_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramReel {
    pub id: String,
    pub shortcode: String,
    pub display_url: String,
    pub video_url: Option<String>,
    pub caption: Option<String>,
    pub views_count: Option<u64>,
    pub likes_count: Option<u64>,
    pub comments_count: Option<u64>,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramUserStats {
    pub posts_count: Option<u64>,
    pub followers_count: Option<u64>,
    pub following_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramUser {
    pub username: String,
    pub full_name: Option<String>,
    pub biography: Option<String>,
    pub profile_pic_url: Option<String>,
    pub is_private: bool,
    pub is_verified: bool,
    pub external_url: Option<String>,
    pub stats: InstagramUserStats,
    pub posts: Option<Vec<InstagramPost>>,
    pub reels: Option<Vec<InstagramReel>>,
    pub scraped_at: DateTime<Utc>,
    pub posts_limited: bool, // Indicates that the posts array is limited and not complete
}

impl InstagramUser {
    // Check if a URL belongs to this user's content (profile pic, posts, reels)
    pub fn is_content_url(&self, url: &str) -> bool {
        // Helper function to extract key image identifiers from Instagram URLs
        fn extract_instagram_image_id(url: &str) -> Option<String> {
            log::debug!("Extracting ID from URL: {}", url);
            
            // First try to get the filename from the URL path
            if let Some(filename_start) = url.rfind('/') {
                let path_part = &url[filename_start+1..];
                if let Some(query_end) = path_part.find('?') {
                    let filename = &path_part[..query_end];
                    if filename.contains('_') && (filename.ends_with(".jpg") || 
                                                 filename.ends_with(".mp4") || 
                                                 filename.contains(".jpg") || 
                                                 filename.contains(".mp4")) {
                        log::debug!("Extracted filename: {}", filename);
                        return Some(filename.to_string());
                    }
                }
            }
            
            // Try to extract the ig_cache_key which is unique to the image
            if let Some(cache_key_start) = url.find("ig_cache_key=") {
                let cache_key_part = &url[cache_key_start + "ig_cache_key=".len()..];
                if let Some(cache_key_end) = cache_key_part.find('&') {
                    let cache_key = &cache_key_part[..cache_key_end];
                    log::debug!("Extracted cache key: {}", cache_key);
                    return Some(cache_key.to_string());
                } else {
                    log::debug!("Extracted full cache key: {}", cache_key_part);
                    return Some(cache_key_part.to_string());
                }
            }
            
            // Extract the image ID from the URL directly
            // Example: 497961779_18033097154648370_200386581629336489_n.jpg
            if let Some(file_start) = url.rfind('/') {
                let path = &url[file_start+1..];
                if path.contains('_') && path.contains(".jpg") {
                    let parts: Vec<&str> = path.split('_').collect();
                    if parts.len() >= 2 {
                        let image_id = format!("{}_{}", parts[0], parts[1]);
                        log::debug!("Extracted image ID from parts: {}", image_id);
                        return Some(image_id);
                    }
                }
            }
            
            log::debug!("Could not extract ID from URL");
            None
        }
        
        // Compare URLs by their key identifiers when possible
        fn urls_match(url1: &str, url2: &str) -> bool {
            if url1 == url2 {
                log::debug!("URLs match exactly");
                return true;
            }
            
            // Try to match by extracted identifiers
            if let (Some(id1), Some(id2)) = (extract_instagram_image_id(url1), extract_instagram_image_id(url2)) {
                // First try direct or decoded comparison
                let matches = id1 == id2 || url_decode(&id1) == url_decode(&id2);
                log::debug!("Comparing IDs: '{}' vs '{}' -> {}", id1, id2, matches);
                
                if matches {
                    return true;
                }
                
                // Special case for cache keys: strip encoding and just compare the base part
                if id1.contains("=") || id2.contains("=") || id1.contains("%3D") || id2.contains("%3D") || 
                   id1.contains("%253D") || id2.contains("%253D") {
                    
                    // Extract the base number before any = or encoding characters
                    let extract_base = |s: &str| -> String {
                        if let Some(idx) = s.find(|c| c == '=' || c == '%') {
                            s[0..idx].to_string()
                        } else {
                            s.to_string()
                        }
                    };
                    
                    let base1 = extract_base(&id1);
                    let base2 = extract_base(&id2);
                    
                    if !base1.is_empty() && !base2.is_empty() && base1 == base2 {
                        log::debug!("Cache key base parts match: '{}' vs '{}'", base1, base2);
                        return true;
                    }
                }
            }
            
            log::debug!("No matching identifiers found between URLs");
            false
        }
        
        // URL-decode a string (for comparing encoded vs non-encoded keys)
        fn url_decode(s: &str) -> String {
            let result = s.replace("%3D", "=")
                .replace("%253D", "=")  // Handle double encoding
                .replace("%25", "%")
                .replace("%2525", "%")  // Handle double encoding
                .replace("%3A", ":")
                .replace("%253A", ":")  // Handle double encoding
                .replace("%3F", "?")
                .replace("%253F", "?"); // Handle double encoding
            
            log::debug!("URL decode: '{}' -> '{}'", s, result);
            result
        }
        
        // Check profile pic
        if let Some(pic) = self.profile_pic_url.as_ref() {
            if urls_match(pic, url) {
                return true;
            }
        }
        
        // Check posts
        if let Some(posts) = self.posts.as_ref() {
            for post in posts {
                if urls_match(&post.display_url, url) {
                    return true;
                }
                
                if let Some(thumb) = &post.thumbnail_url {
                    if urls_match(thumb, url) {
                        return true;
                    }
                }
                
                if let Some(video) = &post.video_url {
                    if urls_match(video, url) {
                        return true;
                    }
                }
            }
        }
        
        // Check reels
        if let Some(reels) = self.reels.as_ref() {
            for reel in reels {
                if urls_match(&reel.display_url, url) {
                    return true;
                }
                
                if let Some(video) = &reel.video_url {
                    if urls_match(video, url) {
                        return true;
                    }
                }
            }
        }
        
        false
    }
}

// Response wrapper for API to include timing info
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramUserResponse {
    pub data: InstagramUser,
    pub from_cache: bool,
    pub cache_age: Option<u64>, // Age in seconds if from cache
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramPostsResponse {
    pub data: Vec<InstagramPost>,
    pub from_cache: bool,
    pub cache_age: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstagramReelsResponse {
    pub data: Vec<InstagramReel>,
    pub from_cache: bool,
    pub cache_age: Option<u64>,
} 