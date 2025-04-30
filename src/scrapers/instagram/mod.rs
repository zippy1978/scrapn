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
        
        // Build request with appropriate headers to mimic a browser
        let mut request = self.client.get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.5")
            .header("Connection", "keep-alive")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none")
            .header("Sec-Fetch-User", "?1")
            .header("TE", "trailers");
        
        // Add cookies if available in config
        if let Some(cookies) = &self.config.instagram_cookies {
            info!("Using Instagram cookies for authentication (limited to first page of posts)");
            request = request.header("Cookie", cookies);
        }
        
        let response = request.send().await?;
        
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
                
                if let Some(user_json) = json_data.get("graphql").and_then(|g| g.get("user")) {
                    // Extract the initial user data
                    let mut user_data = match self.extract_user_data_from_json(&json_data, username) {
                        Some(user) => user,
                        None => {
                            error!("Failed to extract user data from web API JSON for {}", username);
                            return Err(ScraperError::ParsingError("Failed to extract user data".to_string()));
                        }
                    };
                    
                    // Check if we have empty posts but a non-zero post count (pagination issue)
                    if user_data.posts.as_ref().map_or(false, |p| p.is_empty()) && 
                       user_data.stats.posts_count.unwrap_or(0) > 0 && 
                       self.config.instagram_cookies.is_some() {
                        // Found the pagination issue - empty edges array but posts exist
                        info!("Found pagination issue in web API: empty posts array but count is {}. Trying to fetch first page...", 
                              user_data.stats.posts_count.unwrap_or(0));
                        
                        // Try to get the user ID from the response
                        if let Some(user_id) = user_json.get("id").and_then(|v| v.as_str()) {
                            // Make another request to get the first page of posts
                            if let Ok(posts) = self.fetch_user_posts_paged(user_id, username).await {
                                user_data.posts = Some(posts);
                                
                                // If we got posts, also update reels based on these posts
                                if let Some(posts) = &user_data.posts {
                                    if !posts.is_empty() {
                                        let video_posts: Vec<InstagramReel> = posts.iter()
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
                                            user_data.reels = Some(video_posts);
                                        } else {
                                            user_data.reels = Some(Vec::new());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
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
        
        // Build the request with mobile headers
        let mut request = self.client.get(&url)
            .header("User-Agent", "Instagram 219.0.0.12.117 Android")
            .header("Accept", "application/json")
            .header("X-IG-App-ID", "936619743392459") // This is a widely known app ID
            .header("X-ASBD-ID", "198387")
            .header("X-IG-WWW-Claim", "0");
        
        // Add cookies if available in config
        if let Some(cookies) = &self.config.instagram_cookies {
            info!("Using Instagram cookies for mobile API authentication (limited to first page of posts)");
            request = request.header("Cookie", cookies);
        }
        
        let response = request.send().await?;
        
        let status = response.status();
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            if status == reqwest::StatusCode::NOT_FOUND {
                error!("Profile not found via mobile API: {}. Body: {}", username, body);
                return Err(ScraperError::ProfileNotFound);
            }
            
            if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                error!("Unauthorized access to mobile API (cookies may be required): {}. Body: {}", username, body);
                if body.contains("Please wait a few minutes before you try again") {
                    return Err(ScraperError::RateLimited);
                }
            }
            
            error!("Failed to fetch profile via mobile API, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        // Parse the response
        match response.json::<Value>().await {
            Ok(json_data) => {
                // Log the complete JSON structure if authentication is used
                if self.config.instagram_cookies.is_some() {
                    info!("Mobile API authenticated response structure: {}", 
                          serde_json::to_string_pretty(&json_data)
                          .unwrap_or_else(|_| "Failed to format JSON".to_string()));
                }
                
                if let Some(data) = json_data.get("data").and_then(|d| d.get("user")) {
                    // Check if the profile is private
                    if let Some(is_private) = data.get("is_private").and_then(|v| v.as_bool()) {
                        if is_private {
                            error!("Profile is private: {}", username);
                            return Err(ScraperError::PrivateProfile);
                        }
                    }
                    
                    // Extract user data, but it might have empty posts due to pagination
                    let mut user_data = match self.extract_user_data_from_api_response(data, username) {
                        Some(user) => user,
                        None => {
                            error!("Failed to extract user data from API response for {}", username);
                            return Err(ScraperError::ParsingError("Failed to extract user data".to_string()));
                        }
                    };
                    
                    // Check if we have empty posts but a non-zero post count (pagination issue)
                    if user_data.posts.as_ref().map_or(false, |p| p.is_empty()) && 
                       user_data.stats.posts_count.unwrap_or(0) > 0 && 
                       self.config.instagram_cookies.is_some() {
                        // Found the pagination issue - empty edges array but posts exist
                        info!("Found pagination issue: empty posts array but count is {}. Trying to fetch first page...", 
                              user_data.stats.posts_count.unwrap_or(0));
                        
                        // Try to get the user ID from the response
                        if let Some(user_id) = data.get("id").and_then(|v| v.as_str()) {
                            // Make another request to get the first page of posts
                            if let Ok(posts) = self.fetch_user_posts_paged(user_id, username).await {
                                user_data.posts = Some(posts);
                                
                                // If we got posts, also update reels based on these posts
                                if let Some(posts) = &user_data.posts {
                                    if !posts.is_empty() {
                                        let video_posts: Vec<InstagramReel> = posts.iter()
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
                                            user_data.reels = Some(video_posts);
                                        } else {
                                            user_data.reels = Some(Vec::new());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    info!("Successfully extracted user data from mobile API for {}", username);
                    return Ok(user_data);
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
        
        // Build the request with browser-like headers
        let mut request = self.client.get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.5");
        
        // Add cookies if available in config
        if let Some(cookies) = &self.config.instagram_cookies {
            info!("Using Instagram cookies for HTML scraping (limited to first page of posts)");
            request = request.header("Cookie", cookies);
        }
        
        let response = request.send().await?;
        
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
        let mut posts_limited = false;
        
        // Get stats early so we can use post count later
        let stats = InstagramUserStats {
            posts_count: user.get("edge_owner_to_timeline_media")?.get("count")?.as_u64(),
            followers_count: user.get("edge_followed_by")?.get("count")?.as_u64(),
            following_count: user.get("edge_follow")?.get("count")?.as_u64(),
        };
        
        // We'll still create the user object even for private profiles,
        // just without posts and reels
        if !is_private {
            if let Some(timeline) = user.get("edge_owner_to_timeline_media") {
                // Check if the post count is greater than our limit
                if let Some(count) = timeline.get("count").and_then(|v| v.as_u64()) {
                    if count > 12 { // Instagram typically shows 12 posts per page
                        posts_limited = true;
                        info!("Posts will be limited to first page (about 12 posts) of {} available for {}", 
                                count, username);
                    }
                }
                
                posts = self.extract_posts_from_json(timeline);
                
                // If posts is None but we know there are posts, return an empty array
                if posts.is_none() && stats.posts_count.unwrap_or(0) > 0 {
                    info!("Posts count is {} but no posts were extracted from timeline for {}. Returning empty array.", 
                          stats.posts_count.unwrap_or(0), username);
                    posts = Some(Vec::new());
                    posts_limited = true;
                }
            }
            
            // Reels data might be in a different format or endpoint
            // For simplicity, we'll derive reels from video posts
            if let Some(post_vec) = &posts {
                if !post_vec.is_empty() {
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
                    } else {
                        // If we know the user has posts but none are videos, return empty reels array
                        reels = Some(Vec::new());
                    }
                } else {
                    // Posts is empty array, so reels should be too
                    reels = Some(Vec::new());
                }
            }
        }
        
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
            posts_limited,
        })
    }
    
    fn extract_posts_from_json(&self, timeline: &Value) -> Option<Vec<InstagramPost>> {
        let edges = timeline.get("edges")?.as_array()?;
        let mut posts = Vec::new();
        
        // If edges is empty but there's a count, return an empty array instead of None
        if edges.is_empty() {
            if let Some(count) = timeline.get("count").and_then(|v| v.as_u64()) {
                if count > 0 {
                    info!("Found timeline with {} posts but edges array is empty (pagination). Returning empty posts array.", count);
                    return Some(Vec::new());
                }
            }
        }
        
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
        let mut posts_limited = false;
        
        if !is_private {
            // Check if we should limit posts based on the stats
            if let Some(count) = stats.posts_count {
                if count > 12 { // Instagram typically shows 12 posts per page
                    posts_limited = true;
                    info!("Posts will be limited to first page (about 12 posts) of {} available for {}", 
                            count, username);
                }
            }
            
            // Try different possible locations for post data
            if let Some(timeline) = data.get("edge_owner_to_timeline_media")
                .or_else(|| data.get("edge_felix_video_timeline"))
                .or_else(|| data.get("edge_felix_combined_timeline_media")) {
                
                posts = self.extract_posts_from_json(timeline);
                
                // Log if we couldn't extract posts from timeline
                if posts.is_none() {
                    info!("Timeline found but could not extract posts for {}. Timeline structure: {}", 
                          username, 
                          serde_json::to_string_pretty(timeline)
                              .unwrap_or_else(|_| "Failed to format timeline JSON".to_string()));
                }
            } else if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
                posts = self.extract_posts_from_items(items);
            } else if let Some(feed) = data.get("feed") {
                // Handle new structure in authenticated responses
                if let Some(items) = feed.get("items").and_then(|v| v.as_array()) {
                    info!("Extracting posts from feed.items for {}", username);
                    posts = self.extract_posts_from_items(items);
                } else if let Some(media) = feed.get("media").and_then(|v| v.as_object()) {
                    info!("Extracting posts from feed.media for {}", username);
                    // Convert media object to array for processing
                    let media_items: Vec<Value> = media.values().cloned().collect();
                    if !media_items.is_empty() {
                        posts = self.extract_posts_from_items(&media_items);
                    }
                }
            } else if let Some(recent_posts) = data.get("recent_posts") {
                // Another possible location in authenticated responses
                if let Some(items) = recent_posts.get("items").and_then(|v| v.as_array()) {
                    info!("Extracting posts from recent_posts.items for {}", username);
                    posts = self.extract_posts_from_items(items);
                }
            } else {
                // Log that we couldn't find any posts data
                info!("Could not find any posts data structure for {}", username);
            }
            
            // If we still don't have posts, try to look for alternate structures
            if posts.is_none() {
                // Try to find any array property that might contain posts
                for (key, value) in data.as_object()? {
                    if key.contains("media") || key.contains("post") || key.contains("timeline") {
                        if let Some(items) = value.as_array() {
                            if !items.is_empty() {
                                info!("Attempting to extract posts from '{}' property", key);
                                let extracted = self.extract_posts_from_items(items);
                                if extracted.is_some() {
                                    posts = extracted;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            
            // If we still don't have posts but we know the count is > 0, return an empty array
            if posts.is_none() && stats.posts_count.unwrap_or(0) > 0 {
                info!("Posts count is {} but no posts were found in the response for {}. Returning empty array.", 
                      stats.posts_count.unwrap_or(0), username);
                posts = Some(Vec::new());
                posts_limited = true;
            }
            
            // Extract reels - similar to posts extraction but filtering for video content
            // First try to get reels directly if available
            if let Some(reels_data) = data.get("edge_felix_video_timeline")
                .or_else(|| data.get("edge_felix_combined_timeline_media"))
                .or_else(|| data.get("reels_media")) {
                
                let extracted = self.extract_posts_from_json(reels_data);
                if let Some(reels_vec) = extracted {
                    reels = Some(reels_vec.into_iter()
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
                                .collect());
                }
            }
            
            // If no reels found directly, derive from posts
            if reels.is_none() && posts.is_some() {
                let post_vec = posts.as_ref().unwrap();
                if !post_vec.is_empty() {
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
                    } else {
                        // If we have posts but none are videos, initialize reels as an empty array
                        reels = Some(Vec::new());
                    }
                } else {
                    // Posts is an empty array, so reels should be too
                    reels = Some(Vec::new());
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
            posts_limited,
        })
    }
    
    fn extract_posts_from_items(&self, items: &[Value]) -> Option<Vec<InstagramPost>> {
        let mut posts = Vec::new();
        
        for item in items {
            // Log the first item for debugging if it's a complex structure
            if posts.is_empty() && item.is_object() && item.as_object().unwrap().len() > 5 {
                info!("Post item structure sample: {}", 
                      serde_json::to_string_pretty(item)
                          .unwrap_or_else(|_| "Failed to format item JSON".to_string()));
            }
            
            // Extract ID - try multiple possible locations
            let id_str = item.get("id").and_then(|v| v.as_str())
                .or_else(|| item.get("pk").and_then(|v| v.as_str()))
                .or_else(|| item.get("media_id").and_then(|v| v.as_str()))
                .or_else(|| item.get("carousel_media_id").and_then(|v| v.as_str()));
                
            // If we didn't find a string ID, try numeric ID and convert to string
            let id = if let Some(id_val) = id_str {
                id_val.to_string()
            } else if let Some(num_id) = item.get("id").and_then(|v| v.as_u64())
                .or_else(|| item.get("pk").and_then(|v| v.as_u64())) {
                num_id.to_string()
            } else {
                info!("Could not extract ID from post item");
                continue;
            };
            
            // Extract shortcode - try multiple possible paths
            let shortcode = item.get("code").and_then(|v| v.as_str())
                .or_else(|| item.get("shortcode").and_then(|v| v.as_str()))
                .or_else(|| {
                    // Sometimes the shortcode might be in a media object
                    item.get("media").and_then(|m| m.get("code").and_then(|v| v.as_str()))
                });
            
            if shortcode.is_none() {
                info!("Could not extract shortcode for post ID: {}", id);
                continue;
            }
            
            let shortcode = shortcode.unwrap().to_string();
            
            // Determine if the post is a video
            let is_video = item.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false)
                || item.get("media_type").and_then(|v| v.as_u64()).unwrap_or(1) == 2
                || item.get("product_type").and_then(|v| v.as_str()).unwrap_or("") == "clips"
                || item.get("product_type").and_then(|v| v.as_str()).unwrap_or("") == "igtv"
                || item.get("media").and_then(|m| m.get("media_type").and_then(|v| v.as_u64())).unwrap_or(1) == 2;
            
            // Extract display URL (main image) - this can be in many different places
            let display_url = item.get("display_url").and_then(|v| v.as_str())
                .or_else(|| item.get("image_versions2")
                    .and_then(|v| v.get("candidates"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("url").and_then(|u| u.as_str())))
                .or_else(|| item.get("carousel_media")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("image_versions2"))
                    .and_then(|v| v.get("candidates"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("url").and_then(|u| u.as_str())))
                .or_else(|| item.get("media")
                    .and_then(|m| m.get("image_versions2"))
                    .and_then(|v| v.get("candidates"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("url").and_then(|u| u.as_str())))
                .or_else(|| item.get("media")
                    .and_then(|m| m.get("thumbnail_url"))
                    .and_then(|v| v.as_str()))
                .unwrap_or("https://example.com/placeholder.jpg")
                .to_string();
            
            // Extract thumbnail URL - sometimes different from display URL
            let thumbnail_url = item.get("thumbnail_src").and_then(|v| v.as_str())
                .or_else(|| item.get("thumbnail_resources")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("src").and_then(|s| s.as_str())))
                .or_else(|| item.get("thumbnail_url").and_then(|v| v.as_str()))
                .or_else(|| item.get("media")
                    .and_then(|m| m.get("thumbnail_url"))
                    .and_then(|v| v.as_str()))
                .map(str::to_string);
            
            // Extract caption
            let caption = item.get("caption").and_then(|v| {
                if v.is_string() { 
                    v.as_str().map(str::to_string) 
                } else { 
                    v.get("text").and_then(|v| v.as_str()).map(str::to_string) 
                }
            }).or_else(|| {
                // Try alternative paths for caption
                item.get("media")
                    .and_then(|m| m.get("caption"))
                    .and_then(|v| {
                        if v.is_string() { 
                            v.as_str().map(str::to_string) 
                        } else { 
                            v.get("text").and_then(|v| v.as_str()).map(str::to_string) 
                        }
                    })
            });
            
            // Extract likes count
            let likes_count = item.get("like_count").and_then(|v| v.as_u64())
                .or_else(|| item.get("likes").and_then(|v| v.get("count")).and_then(|v| v.as_u64()))
                .or_else(|| item.get("media")
                    .and_then(|m| m.get("like_count"))
                    .and_then(|v| v.as_u64()));
            
            // Extract comments count
            let comments_count = item.get("comment_count").and_then(|v| v.as_u64())
                .or_else(|| item.get("comments").and_then(|v| v.get("count")).and_then(|v| v.as_u64()))
                .or_else(|| item.get("comments_count").and_then(|v| v.as_u64()))
                .or_else(|| item.get("media")
                    .and_then(|m| m.get("comment_count"))
                    .and_then(|v| v.as_u64()));
            
            // Extract timestamp
            let timestamp = item.get("taken_at").and_then(|v| v.as_i64())
                .or_else(|| item.get("taken_at_timestamp").and_then(|v| v.as_i64()))
                .or_else(|| item.get("created_time").and_then(|v| v.as_i64()))
                .or_else(|| item.get("media")
                    .and_then(|m| m.get("taken_at"))
                    .and_then(|v| v.as_i64()))
                .map(|ts| Utc.timestamp_opt(ts, 0).single())
                .flatten();
            
            // Extract video URL and view count if it's a video
            let video_url = if is_video {
                item.get("video_url").and_then(|v| v.as_str())
                    .or_else(|| item.get("media")
                        .and_then(|m| m.get("video_url"))
                        .and_then(|v| v.as_str()))
                    .map(str::to_string)
            } else {
                None
            };
            
            let video_view_count = if is_video {
                item.get("view_count").and_then(|v| v.as_u64())
                    .or_else(|| item.get("play_count").and_then(|v| v.as_u64()))
                    .or_else(|| item.get("video_view_count").and_then(|v| v.as_u64()))
                    .or_else(|| item.get("media")
                        .and_then(|m| m.get("view_count"))
                        .and_then(|v| v.as_u64()))
            } else {
                None
            };
            
            // Create and add the post
            let post = InstagramPost {
                id,
                shortcode,
                display_url,
                thumbnail_url,
                caption,
                likes_count,
                comments_count,
                timestamp,
                is_video,
                video_url,
                video_view_count,
            };
            
            posts.push(post);
        }
        
        if posts.is_empty() {
            None
        } else {
            Some(posts)
        }
    }
    
    // Method to fetch a specific page of posts for a user
    async fn fetch_user_posts_paged(&self, user_id: &str, username: &str) -> Result<Vec<InstagramPost>, ScraperError> {
        // Construct the query hash URL - this is used to fetch user posts with pagination
        // Try a different query hash that's known to work for authenticated requests
        let url = format!(
            "https://www.instagram.com/graphql/query/?query_hash=69cba40317214236af40e7efa697781d&variables=%7B%22id%22%3A%22{}%22%2C%22first%22%3A12%7D",
            user_id
        );
        
        info!("Fetching first page of posts for {} (user ID: {})", username, user_id);
        
        // Build request with appropriate headers
        let mut request = self.client.get(&url)
            .header("Accept", "application/json")
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
            .header("Referer", format!("https://www.instagram.com/{}/", username))
            .header("X-Requested-With", "XMLHttpRequest")
            .header("X-IG-App-ID", "936619743392459");
            
        // Add cookies if available
        if let Some(cookies) = &self.config.instagram_cookies {
            request = request.header("Cookie", cookies);
        }
        
        let response = request.send().await?;
        let status = response.status();
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch posts page, status: {}. Response body: {}", status, body);
            return Ok(Vec::new()); // Return empty rather than error to keep profile info
        }
        
        // Get the response body as text first to inspect it
        let response_text = match response.text().await {
            Ok(text) => {
                // Print the first 500 characters to avoid overwhelming logs
                let preview = if text.len() > 500 {
                    format!("{}... (truncated)", &text[..500])
                } else {
                    text.clone()
                };
                info!("Pagination response preview: {}", preview);
                text
            },
            Err(e) => {
                error!("Failed to read pagination response text: {}", e);
                return Ok(Vec::new());
            }
        };
        
        // Try alternate pagination endpoint if response isn't JSON
        if !response_text.trim().starts_with('{') {
            info!("Response doesn't look like JSON, trying alternate pagination approach");
            return self.fetch_user_posts_alternate(user_id, username).await;
        }
        
        // Now parse the JSON
        match serde_json::from_str::<Value>(&response_text) {
            Ok(json_data) => {
                // Try to extract posts from the pagination response
                if let Some(edge_owner_to_timeline_media) = json_data
                    .get("data")
                    .and_then(|d| d.get("user"))
                    .and_then(|u| u.get("edge_owner_to_timeline_media")) {
                    
                    if let Some(extracted_posts) = self.extract_posts_from_json(edge_owner_to_timeline_media) {
                        info!("Successfully extracted {} posts from pagination request", extracted_posts.len());
                        return Ok(extracted_posts);
                    }
                } else {
                    // Log the response to diagnose issues
                    info!("Pagination response doesn't contain expected structure");
                }
            },
            Err(e) => {
                error!("Failed to parse paginated posts response: {}. Response starts with: '{}'", 
                      e, response_text.chars().take(30).collect::<String>());
            }
        }
        
        // If we get here, try another approach as fallback
        self.fetch_user_posts_alternate(user_id, username).await
    }
    
    // Alternative method for fetching posts if the GraphQL approach fails
    async fn fetch_user_posts_alternate(&self, user_id: &str, username: &str) -> Result<Vec<InstagramPost>, ScraperError> {
        // Try using a more reliable endpoint that returns the user's posts
        let url = format!("https://i.instagram.com/api/v1/feed/user/{}/", user_id);
        
        info!("Trying alternate method to fetch posts for {} (user ID: {})", username, user_id);
        
        let mut request = self.client.get(&url)
            .header("User-Agent", "Instagram 219.0.0.12.117 Android")
            .header("Accept", "application/json")
            .header("X-IG-App-ID", "936619743392459")
            .header("X-ASBD-ID", "198387")
            .header("X-IG-WWW-Claim", "0");
            
        // Add cookies if available
        if let Some(cookies) = &self.config.instagram_cookies {
            request = request.header("Cookie", cookies);
        }
        
        let response = request.send().await?;
        let status = response.status();
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch posts with alternate method, status: {}. Response body: {}", 
                  status, body);
            return Ok(Vec::new());
        }
        
        // Get the response as text first
        let response_text = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                error!("Failed to read alternate pagination response text: {}", e);
                return Ok(Vec::new());
            }
        };
        
        // Now try to parse as JSON
        match serde_json::from_str::<Value>(&response_text) {
            Ok(json_data) => {
                // Try to extract items from the mobile API response
                if let Some(items) = json_data.get("items").and_then(|v| v.as_array()) {
                    if let Some(extracted_posts) = self.extract_posts_from_items(items) {
                        info!("Successfully extracted {} posts using alternate method", extracted_posts.len());
                        return Ok(extracted_posts);
                    }
                } else {
                    // Log brief response for debugging
                    let preview = if response_text.len() > 200 {
                        format!("{}... (truncated)", &response_text[..200])
                    } else {
                        response_text.clone()
                    };
                    info!("Alternate method response doesn't contain items array. Preview: {}", preview);
                }
            },
            Err(e) => {
                error!("Failed to parse alternate method response: {}", e);
            }
        }
        
        // Return empty array if all attempts fail
        Ok(Vec::new())
    }
} 