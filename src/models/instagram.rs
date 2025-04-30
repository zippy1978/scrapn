use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

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