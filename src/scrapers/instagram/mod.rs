use scraper::{Html, Selector};
use reqwest::Client;
use regex::Regex;
use serde_json::{Value, json};
use chrono::{Utc, TimeZone};
use std::time::Duration;
use thiserror::Error;
use log::{info, error};

use crate::models::instagram::{
    InstagramUser, InstagramPost, InstagramReel, InstagramUserStats
};
use crate::config::AppConfig;

#[derive(Error, Debug)]
pub enum ScraperError {
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),
    
    #[error("Parsing error: {0}")]
    ParsingError(String),
    
    #[error("Rate limited or blocked")]
    RateLimited,
    
    #[error("Profile not found")]
    ProfileNotFound,
    
    #[error("Private profile")]
    PrivateProfile,
}

pub struct InstagramScraper {
    client: Client,
    config: AppConfig,
}

impl InstagramScraper {
    pub fn new(config: AppConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout))
            .user_agent(&config.user_agent)
            .build()
            .expect("Failed to build HTTP client");
            
        Self { client, config }
    }
    
    pub async fn scrape_user(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        info!("Scraping Instagram user: {}", username);
        
        // Use the timeout from config to validate it's being used
        let _timeout = self.config.timeout;
        
        // First attempt: Try the web API endpoint
        if let Ok(user) = self.try_web_api_endpoint(username).await {
            return Ok(user);
        }
        
        // Second attempt: Try the mobile API endpoint
        if let Ok(user) = self.try_mobile_api_endpoint(username).await {
            return Ok(user);
        }
        
        // Third attempt: Try HTML scraping
        if let Ok(user) = self.try_html_scraping(username).await {
            return Ok(user);
        }
        
        // If all attempts fail, return an error
        error!("Could not extract data for {}", username);
        Err(ScraperError::ParsingError(format!("Could not extract data for {}", username)))
    }
    
    async fn try_web_api_endpoint(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        // Request the user's profile page using the API-like endpoint
        let url = format!("https://www.instagram.com/{}/?__a=1&__d=dis", username);
        
        info!("Trying web API endpoint for {}", username);
        
        // Send request with appropriate headers to mimic a browser
        let response = self.client.get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.5")
            .header("Connection", "keep-alive")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .header("TE", "trailers")
            .send()
            .await?;
        
        let status = response.status();
        
        if status == reqwest::StatusCode::NOT_FOUND {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Profile not found: {}. Body: {}", username, body);
            return Err(ScraperError::ProfileNotFound);
        }
        
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Rate limited by Instagram. Body: {}", body);
            return Err(ScraperError::RateLimited);
        }
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch profile, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        // Try to get JSON data using the API-like endpoint
        match response.json::<Value>().await {
            Ok(json_data) => {
                // Check if the profile is private
                if let Some(is_private) = json_data.get("graphql")
                    .and_then(|g| g.get("user"))
                    .and_then(|u| u.get("is_private"))
                    .and_then(|p| p.as_bool()) 
                {
                    if is_private {
                        error!("Profile is private: {}", username);
                        return Err(ScraperError::PrivateProfile);
                    }
                }
                
                if let Some(user_data) = self.extract_user_data_from_json(&json_data, username) {
                    info!("Successfully extracted user data from web API for {}", username);
                    return Ok(user_data);
                }
            },
            Err(e) => {
                error!("Failed to parse JSON response: {}. Error: {}", username, e);
            }
        }
        
        Err(ScraperError::ParsingError("Could not extract data from web API".to_string()))
    }
    
    async fn try_mobile_api_endpoint(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        // Try to use the mobile API-like endpoint which sometimes has different data
        let url = format!("https://i.instagram.com/api/v1/users/web_profile_info/?username={}", username);
        
        info!("Trying mobile API endpoint for {}", username);
        
        let response = self.client.get(&url)
            .header("User-Agent", "Instagram 219.0.0.12.117 Android")
            .header("Accept", "application/json")
            .header("X-IG-App-ID", "936619743392459") // This is a widely known app ID
            .header("X-ASBD-ID", "198387")
            .header("X-IG-WWW-Claim", "0")
            .send()
            .await?;
        
        let status = response.status();
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            if status == reqwest::StatusCode::NOT_FOUND {
                error!("Profile not found via mobile API: {}. Body: {}", username, body);
                return Err(ScraperError::ProfileNotFound);
            }
            
            error!("Failed to fetch profile via mobile API, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        // Parse the response
        match response.json::<Value>().await {
            Ok(json_data) => {
                if let Some(data) = json_data.get("data").and_then(|d| d.get("user")) {
                    // Check if the profile is private
                    if let Some(is_private) = data.get("is_private").and_then(|v| v.as_bool()) {
                        if is_private {
                            error!("Profile is private: {}", username);
                            return Err(ScraperError::PrivateProfile);
                        }
                    }
                    
                    if let Some(user) = self.extract_user_data_from_api_response(data, username) {
                        info!("Successfully extracted user data from mobile API for {}", username);
                        return Ok(user);
                    }
                } else {
                    error!("User data not found in response. Full JSON: {}", json_data);
                }
            },
            Err(e) => {
                error!("Failed to parse mobile API JSON response: {}. Error: {}", username, e);
            }
        }
        
        Err(ScraperError::ParsingError("Could not extract data from mobile API".to_string()))
    }
    
    async fn try_html_scraping(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        // Fallback to HTML scraping if API approaches fail
        info!("Falling back to HTML scraping for {}", username);
        let url = format!("https://www.instagram.com/{}/", username);
        
        let response = self.client.get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.5")
            .send()
            .await?;
        
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch profile HTML, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        let html = response.text().await?;
        
        // Try different methods to extract the data
        if let Some(user_data) = self.extract_from_shared_data(&html, username) {
            info!("Successfully extracted user data from HTML for {}", username);
            return Ok(user_data);
        }
        
        // Log a portion of the HTML to help debug extraction failures
        let preview_length = std::cmp::min(500, html.len());
        error!("Failed to extract user data from HTML. Preview of HTML: {}...", &html[..preview_length]);
        
        Err(ScraperError::ParsingError("Could not extract data from HTML".to_string()))
    }
    
    fn extract_user_data_from_json(&self, data: &Value, username: &str) -> Option<InstagramUser> {
        // This handles the JSON format for the ?__a=1&__d=dis endpoint
        let user = data.get("graphql")?.get("user")?;
        
        let now = Utc::now();
        let is_private = user.get("is_private")?.as_bool()?;
        
        let mut posts = None;
        let mut reels = None;
        
        // We'll still create the user object even for private profiles,
        // just without posts and reels
        if !is_private {
            if let Some(timeline) = user.get("edge_owner_to_timeline_media") {
                posts = self.extract_posts_from_json(timeline);
            }
            
            // Reels data might be in a different format or endpoint
            // For simplicity, we'll derive reels from video posts
            if let Some(post_vec) = &posts {
                let video_posts: Vec<InstagramReel> = post_vec.iter()
                    .filter(|post| post.is_video)
                    .map(|post| InstagramReel {
                        id: post.id.clone(),
                        shortcode: post.shortcode.clone(),
                        display_url: post.display_url.clone(),
                        video_url: post.video_url.clone(),
                        caption: post.caption.clone(),
                        views_count: post.video_view_count,
                        likes_count: post.likes_count,
                        comments_count: post.comments_count,
                        timestamp: post.timestamp,
                    })
                    .collect();
                
                if !video_posts.is_empty() {
                    reels = Some(video_posts);
                }
            }
        }
        
        let stats = InstagramUserStats {
            posts_count: user.get("edge_owner_to_timeline_media")?.get("count")?.as_u64(),
            followers_count: user.get("edge_followed_by")?.get("count")?.as_u64(),
            following_count: user.get("edge_follow")?.get("count")?.as_u64(),
        };
        
        Some(InstagramUser {
            username: username.to_string(),
            full_name: user.get("full_name").and_then(|v| v.as_str()).map(str::to_string),
            biography: user.get("biography").and_then(|v| v.as_str()).map(str::to_string),
            profile_pic_url: user.get("profile_pic_url_hd")
                .or_else(|| user.get("profile_pic_url"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            is_private,
            is_verified: user.get("is_verified").and_then(|v| v.as_bool()).unwrap_or(false),
            external_url: user.get("external_url").and_then(|v| v.as_str()).map(str::to_string),
            stats,
            posts,
            reels,
            scraped_at: now,
        })
    }
    
    fn extract_posts_from_json(&self, timeline: &Value) -> Option<Vec<InstagramPost>> {
        let edges = timeline.get("edges")?.as_array()?;
        let mut posts = Vec::new();
        
        for edge in edges {
            let node = edge.get("node")?;
            
            let post = InstagramPost {
                id: node.get("id")?.as_str()?.to_string(),
                shortcode: node.get("shortcode")?.as_str()?.to_string(),
                display_url: node.get("display_url")?.as_str()?.to_string(),
                thumbnail_url: node.get("thumbnail_src").and_then(|v| v.as_str()).map(str::to_string),
                caption: node.get("edge_media_to_caption")
                    .and_then(|v| v.get("edges"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("node"))
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                likes_count: node.get("edge_liked_by")
                    .and_then(|v| v.get("count"))
                    .and_then(|v| v.as_u64()),
                comments_count: node.get("edge_media_to_comment")
                    .and_then(|v| v.get("count"))
                    .and_then(|v| v.as_u64()),
                timestamp: node.get("taken_at_timestamp")
                    .and_then(|v| v.as_i64())
                    .map(|ts| Utc.timestamp_opt(ts, 0).single())
                    .flatten(),
                is_video: node.get("is_video")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                video_url: if node.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false) {
                    node.get("video_url").and_then(|v| v.as_str()).map(str::to_string)
                } else {
                    None
                },
                video_view_count: if node.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false) {
                    node.get("video_view_count").and_then(|v| v.as_u64())
                } else {
                    None
                },
            };
            
            posts.push(post);
        }
        
        if posts.is_empty() {
            None
        } else {
            Some(posts)
        }
    }
    
    fn extract_from_shared_data(&self, html: &str, username: &str) -> Option<InstagramUser> {
        let re = Regex::new(r#"window\._sharedData = (.+?);</script>"#).ok()?;
        let caps = re.captures(html)?;
        
        if let Ok(json) = serde_json::from_str::<Value>(&caps[1]) {
            let user_json = json
                .get("entry_data")?
                .get("ProfilePage")?
                .get(0)?
                .get("graphql")?
                .get("user")?;
            
            return self.extract_user_data_from_json(&json!({ "graphql": { "user": user_json } }), username);
        }
        
        // Try alternative approaches if the above fails
        if let Some(user) = self.extract_from_additional_data_sources(html, username) {
            return Some(user);
        }
        
        None
    }
    
    fn extract_from_additional_data_sources(&self, html: &str, username: &str) -> Option<InstagramUser> {
        // Try to find additional JSON data patterns in the page
        // Instagram keeps changing their data patterns, so we need multiple approaches
        
        // Try to extract from window.__additionalDataLoaded
        let additional_data_re = Regex::new(r#"window\.__additionalDataLoaded\s*\(\s*['"].*?['"]\s*,\s*(.+?)\);"#).ok()?;
        if let Some(caps) = additional_data_re.captures(html) {
            if let Ok(json) = serde_json::from_str::<Value>(&caps[1]) {
                if let Some(user_json) = json.get("user") {
                    return self.extract_user_data_from_api_response(user_json, username);
                }
            }
        }
        
        // Try to extract from a newer pattern - look for script with type="application/json"
        let html_doc = Html::parse_document(html);
        let script_selector = Selector::parse("script[type='application/json']").ok()?;
        
        for script in html_doc.select(&script_selector) {
            if let Some(content) = script.text().next() {
                if let Ok(json) = serde_json::from_str::<Value>(content) {
                    // Look for user data in various locations within the JSON
                    if let Some(data) = json.get("require")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.iter().find(|item| 
                            item.get(0).and_then(|v| v.as_str()).unwrap_or("") == "ProfilePageContainer"
                        ))
                        .and_then(|item| item.get(3))
                        .and_then(|v| v.get("user")) {
                        
                        return self.extract_user_data_from_api_response(data, username);
                    }
                }
            }
        }
        
        None
    }
    
    fn extract_user_data_from_api_response(&self, data: &Value, username: &str) -> Option<InstagramUser> {
        // Handle data format from API-like responses that differ from graphql
        let now = Utc::now();
        
        let is_private = data.get("is_private").and_then(|v| v.as_bool()).unwrap_or(false);
        
        // Extract stats
        let stats = InstagramUserStats {
            posts_count: data.get("media_count").and_then(|v| v.as_u64())
                .or_else(|| data.get("edge_owner_to_timeline_media").and_then(|v| v.get("count")).and_then(|v| v.as_u64())),
            followers_count: data.get("follower_count").and_then(|v| v.as_u64())
                .or_else(|| data.get("edge_followed_by").and_then(|v| v.get("count")).and_then(|v| v.as_u64())),
            following_count: data.get("following_count").and_then(|v| v.as_u64())
                .or_else(|| data.get("edge_follow").and_then(|v| v.get("count")).and_then(|v| v.as_u64())),
        };
        
        // Extract posts and reels if available
        let mut posts = None;
        let mut reels = None;
        
        if !is_private {
            // Try different possible locations for post data
            if let Some(timeline) = data.get("edge_owner_to_timeline_media")
                .or_else(|| data.get("edge_felix_video_timeline"))
                .or_else(|| data.get("edge_felix_combined_timeline_media")) {
                
                posts = self.extract_posts_from_json(timeline);
            } else if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
                posts = self.extract_posts_from_items(items);
            }
            
            // Extract reels - similar to posts extraction but filtering for video content
            if let Some(post_vec) = &posts {
                let video_posts: Vec<InstagramReel> = post_vec.iter()
                    .filter(|post| post.is_video)
                    .map(|post| InstagramReel {
                        id: post.id.clone(),
                        shortcode: post.shortcode.clone(),
                        display_url: post.display_url.clone(),
                        video_url: post.video_url.clone(),
                        caption: post.caption.clone(),
                        views_count: post.video_view_count,
                        likes_count: post.likes_count,
                        comments_count: post.comments_count,
                        timestamp: post.timestamp,
                    })
                    .collect();
                
                if !video_posts.is_empty() {
                    reels = Some(video_posts);
                }
            }
        }
        
        Some(InstagramUser {
            username: username.to_string(),
            full_name: data.get("full_name").and_then(|v| v.as_str()).map(str::to_string),
            biography: data.get("biography").and_then(|v| v.as_str()).map(str::to_string),
            profile_pic_url: data.get("profile_pic_url_hd")
                .or_else(|| data.get("profile_pic_url"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            is_private,
            is_verified: data.get("is_verified").and_then(|v| v.as_bool()).unwrap_or(false),
            external_url: data.get("external_url").and_then(|v| v.as_str()).map(str::to_string),
            stats,
            posts,
            reels,
            scraped_at: now,
        })
    }
    
    fn extract_posts_from_items(&self, items: &[Value]) -> Option<Vec<InstagramPost>> {
        let mut posts = Vec::new();
        
        for item in items {
            let id = item.get("id").and_then(|v| v.as_str()).or_else(|| 
                item.get("pk").and_then(|v| v.as_str())
            );
            
            if let Some(id) = id {
                let shortcode = item.get("code").and_then(|v| v.as_str())
                    .or_else(|| item.get("shortcode").and_then(|v| v.as_str()));
                
                if let Some(shortcode) = shortcode {
                    let is_video = item.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false)
                        || item.get("media_type").and_then(|v| v.as_u64()).unwrap_or(1) == 2;
                    
                    let post = InstagramPost {
                        id: id.to_string(),
                        shortcode: shortcode.to_string(),
                        display_url: item.get("display_url")
                            .or_else(|| item.get("image_versions2")
                                .and_then(|v| v.get("candidates"))
                                .and_then(|v| v.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|v| v.get("url")))
                            .and_then(|v| v.as_str())
                            .unwrap_or("https://example.com/placeholder.jpg")
                            .to_string(),
                        thumbnail_url: item.get("thumbnail_src")
                            .or_else(|| item.get("thumbnail_resources")
                                .and_then(|v| v.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|v| v.get("src")))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        caption: item.get("caption")
                            .and_then(|v| if v.is_string() { 
                                v.as_str().map(str::to_string) 
                            } else { 
                                v.get("text").and_then(|v| v.as_str()).map(str::to_string) 
                            }),
                        likes_count: item.get("like_count")
                            .or_else(|| item.get("like_count"))
                            .and_then(|v| v.as_u64()),
                        comments_count: item.get("comment_count")
                            .or_else(|| item.get("comments")
                                .and_then(|v| v.get("count")))
                            .and_then(|v| v.as_u64()),
                        timestamp: item.get("taken_at")
                            .or_else(|| item.get("taken_at_timestamp"))
                            .and_then(|v| v.as_i64())
                            .map(|ts| Utc.timestamp_opt(ts, 0).single())
                            .flatten(),
                        is_video,
                        video_url: if is_video {
                            item.get("video_url")
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        } else {
                            None
                        },
                        video_view_count: if is_video {
                            item.get("view_count")
                                .or_else(|| item.get("play_count"))
                                .and_then(|v| v.as_u64())
                        } else {
                            None
                        },
                    };
                    
                    posts.push(post);
                }
            }
        }
        
        if posts.is_empty() {
            None
        } else {
            Some(posts)
        }
    }
} 