use scraper::{Html, Selector};
use reqwest::{Client, Proxy};
use regex::Regex;
use serde_json::Value;
use chrono::{Utc, TimeZone};
use std::time::Duration;
use thiserror::Error;
use log::{info, error, warn, debug};

use crate::models::instagram::{
    InstagramUser, InstagramPost, InstagramReel, InstagramUserStats
};
use crate::config::AppConfig;
use crate::proxy::ProxyManager;

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
    
    #[error("Proxy error: {0}")]
    ProxyError(String),
    
    #[error("All proxies failed")]
    AllProxiesFailed,
    
    #[error("Unauthorized access: {0}")]
    UnauthorizedAccess(String),
}

pub struct InstagramScraper {
    config: AppConfig,
    proxy_manager: Option<ProxyManager>,
}

impl InstagramScraper {
    pub fn new(config: AppConfig, proxy_manager: ProxyManager) -> Self {
        Self { 
            config,
            proxy_manager: Some(proxy_manager),
        }
    }
  
    pub async fn scrape_user(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        info!("Scraping Instagram user: {}", username);
        
        // First attempt: Try the web API endpoint with proxy rotation
        match self.try_web_api_endpoint(username).await {
            Ok(user) => return Ok(user),
            Err(ScraperError::AllProxiesFailed) => {
                warn!("All proxies failed for web API endpoint, trying mobile API endpoint");
            },
            Err(e) => {
                warn!("Web API endpoint failed: {}, trying mobile API endpoint", e);
            }
        }
        
        // Second attempt: Try the mobile API endpoint
        match self.try_mobile_api_endpoint(username).await {
            Ok(user) => return Ok(user),
            Err(ScraperError::AllProxiesFailed) => {
                warn!("All proxies failed for mobile API endpoint, trying HTML scraping");
            },
            Err(e) => {
                warn!("Mobile API endpoint failed: {}, trying HTML scraping", e);
            }
        }
        
        // Third attempt: Try HTML scraping
        match self.try_html_scraping(username).await {
            Ok(user) => return Ok(user),
            Err(e) => {
                error!("HTML scraping failed: {}", e);
                return Err(e);
            }
        }
    }
    
    async fn try_web_api_endpoint(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        // Request the user's profile page using the API-like endpoint
        let url = format!("https://www.instagram.com/{}/?__a=1&__d=dis", username);
        
        info!("Trying web API endpoint for {}", username);
        
        if let Some(proxy_manager) = &self.proxy_manager {
            // Try with each proxy until one works or all fail
            let mut last_error = None;
            
            // Get proxy count to know how many to try
            let (available, total) = proxy_manager.get_proxy_count();
            
            // If no proxies are available, return error - don't try without proxy
            if available == 0 {
                if total > 0 {
                    warn!("No proxies available (all marked as unavailable), not falling back to direct connection");
                    return Err(ScraperError::AllProxiesFailed);
                } else {
                    warn!("No proxies configured");
                    return Err(ScraperError::ProxyError("No proxies configured".to_string()));
                }
            }
            
            // Try up to available_proxies number of proxies
            for _ in 0..available {
                if let Some(proxy_url) = proxy_manager.get_random_proxy() {
                    info!("Trying request with proxy: {}", proxy_url);
                    
                    match self.make_api_request(&url, username, Some(&proxy_url)).await {
                        Ok(result) => {
                            return Ok(result);
                        }
                        Err(err) => {
                            // If it's a proxy error, mark this proxy as unavailable
                            if let ScraperError::ProxyError(msg) = &err {
                                warn!("Proxy error: {}, marking proxy as unavailable", msg);
                                proxy_manager.mark_proxy_unavailable(&proxy_url);
                            }
                            last_error = Some(err);
                        }
                    }
                }
            }
            
            // If we reached here, all proxies failed
            if let Some(err) = last_error {
                warn!("All proxies failed: {}", err);
            }
            return Err(ScraperError::AllProxiesFailed);
        } else {
            // No proxy manager, use the default client
            return self.make_api_request(&url, username, None).await;
        }
    }
    
    async fn make_api_request(&self, url: &str, username: &str, proxy_url: Option<&str>) -> Result<InstagramUser, ScraperError> {
        let client_builder = Client::builder()
            .timeout(Duration::from_secs(self.config.timeout))
            .user_agent(&self.config.user_agent);
            
        // Add proxy if provided
        let client_builder = if let Some(proxy) = proxy_url {
            if let Some(proxy_manager) = &self.proxy_manager {
                // Use the normalized proxy URL with explicit protocol
                let normalized_proxy = proxy_manager.normalize_proxy_url(proxy);
                info!("Using normalized proxy URL: {}", normalized_proxy);
                match Proxy::all(&normalized_proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            } else {
                // Fallback to original behavior if no proxy manager
                match Proxy::all(proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            }
        } else {
            client_builder
        };
        
        let client = match client_builder.build() {
            Ok(client) => client,
            Err(e) => return Err(ScraperError::ProxyError(format!("Failed to build client: {}", e))),
        };
        
        // Build request with appropriate headers to mimic a browser
        let mut request = client.get(url)
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
        
        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                if let Some(_proxy) = proxy_url {
                    return Err(ScraperError::ProxyError(format!("Proxy request failed: {}", e)));
                }
                return Err(ScraperError::NetworkError(e));
            }
        };
        
        let status = response.status();
        
        // Log headers for debugging
        self.log_response_headers(&response, "web API");
        
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
        match response.text().await {
            Ok(text_body) => {
                if text_body.is_empty() {
                    error!("Empty response body for {}", username);
                    return Err(ScraperError::ParsingError("Empty response body".to_string()));
                }
                
                // Log the response body for debugging
                info!("Web API response body: {}", text_body);
                
                // Try to parse the JSON
                match serde_json::from_str::<Value>(&text_body) {
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
                               self.config.instagram_cookies.is_some()
                            {
                                // We can try to fetch additional posts if we have auth cookies
                                info!("Initial fetch returned no posts but post count > 0. Trying to fetch posts via API...");
                                
                                // Get the user ID for pagination
                                if let Some(user_id) = user_json.get("id").and_then(|id| id.as_str()) {
                                    match self.fetch_user_posts_paged(user_id, username, proxy_url).await {
                                        Ok(posts) => {
                                            user_data.posts = Some(posts);
                                            user_data.posts_limited = true;
                                        },
                                        Err(e) => {
                                            warn!("Failed to fetch additional posts: {}", e);
                                        }
                                    }
                                }
                            }
                            
                            return Ok(user_data);
                        }
                    },
                    Err(e) => {
                        if text_body.trim().is_empty() {
                            error!("Failed to parse JSON response: {}. Error: {}, Response body is empty", username, e);
                        } else if text_body.len() < 100 {
                            // If response is very short, log the full content
                            error!("Failed to parse JSON response: {}. Error: {}, Short response body: {}", username, e, text_body);
                        } else {
                            // For longer responses, log a preview
                            let preview = if text_body.len() > 500 { &text_body[0..500] } else { &text_body };
                            error!("Failed to parse JSON response: {}. Error: {}, Response body preview: {}...", username, e, preview);
                        }
                    }
                }
            },
            Err(e) => {
                error!("Failed to get response body: {}. Error: {}", username, e);
            }
        }
        
        Err(ScraperError::ParsingError("Could not extract data from web API".to_string()))
    }
    
    async fn try_mobile_api_endpoint(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        // Try to fetch user data from the mobile API-like endpoint
        let url = format!("https://i.instagram.com/api/v1/users/web_profile_info/?username={}", username);
        
        info!("Trying mobile API endpoint for {}", username);
        
        if let Some(proxy_manager) = &self.proxy_manager {
            // Try with each proxy until one works or all fail
            let mut last_error = None;
            
            // Get proxy count to know how many to try
            let (available, total) = proxy_manager.get_proxy_count();
            
            // If no proxies are available, return error - don't try without proxy
            if available == 0 {
                if total > 0 {
                    warn!("No proxies available (all marked as unavailable), not falling back to direct connection");
                    return Err(ScraperError::AllProxiesFailed);
                } else {
                    warn!("No proxies configured");
                    return Err(ScraperError::ProxyError("No proxies configured".to_string()));
                }
            }
            
            // Try up to available_proxies number of proxies
            for _ in 0..available {
                if let Some(proxy_url) = proxy_manager.get_random_proxy() {
                    info!("Trying mobile API request with proxy: {}", proxy_url);
                    
                    match self.make_mobile_api_request(&url, username, Some(&proxy_url)).await {
                        Ok(result) => {
                            return Ok(result);
                        }
                        Err(err) => {
                            // If it's a proxy error, mark this proxy as unavailable
                            if let ScraperError::ProxyError(msg) = &err {
                                warn!("Proxy error: {}, marking proxy as unavailable", msg);
                                proxy_manager.mark_proxy_unavailable(&proxy_url);
                            }
                            last_error = Some(err);
                        }
                    }
                }
            }
            
            // If we reached here, all proxies failed
            if let Some(err) = last_error {
                warn!("All proxies failed for mobile API request: {}", err);
            }
            return Err(ScraperError::AllProxiesFailed);
        } else {
            // No proxy manager, use the default client
            return self.make_mobile_api_request(&url, username, None).await;
        }
    }
    
    async fn make_mobile_api_request(&self, url: &str, username: &str, proxy_url: Option<&str>) -> Result<InstagramUser, ScraperError> {
        let client_builder = Client::builder()
            .timeout(Duration::from_secs(self.config.timeout))
            .user_agent("Instagram 76.0.0.15.395 Android (28/9; 420dpi; 1080x2034; OnePlus; ONEPLUS A6003; OnePlus6; qcom; en_US; 139064830)");
            
        // Add proxy if provided
        let client_builder = if let Some(proxy) = proxy_url {
            if let Some(proxy_manager) = &self.proxy_manager {
                // Use the normalized proxy URL with explicit protocol
                let normalized_proxy = proxy_manager.normalize_proxy_url(proxy);
                info!("Using normalized proxy URL: {}", normalized_proxy);
                match Proxy::all(&normalized_proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            } else {
                // Fallback to original behavior if no proxy manager
                match Proxy::all(proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            }
        } else {
            client_builder
        };
        
        let client = match client_builder.build() {
            Ok(client) => client,
            Err(e) => return Err(ScraperError::ProxyError(format!("Failed to build client: {}", e))),
        };
        
        // Build request with mobile API specific headers
        let mut request = client.get(url)
            .header("User-Agent", "Instagram 219.0.0.12.117 Android")
            .header("Accept", "application/json")
            .header("Accept-Language", "en-US")
            .header("X-IG-App-ID", "936619743392459")
            .header("X-ASBD-ID", "198387")
            .header("X-IG-WWW-Claim", "0");
        
        // Add cookies if available in config
        if let Some(cookies) = &self.config.instagram_cookies {
            info!("Using Instagram cookies for mobile API authentication (limited to first page of posts)");
            request = request.header("Cookie", cookies);
        }
        
        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                if let Some(_proxy) = proxy_url {
                    return Err(ScraperError::ProxyError(format!("Proxy request failed: {}", e)));
                }
                return Err(ScraperError::NetworkError(e));
            }
        };
        
        let status = response.status();
        
        // Log headers for debugging
        self.log_response_headers(&response, "mobile API");
        
        if status == reqwest::StatusCode::NOT_FOUND {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Profile not found via mobile API: {}. Body: {}", username, body);
            return Err(ScraperError::ProfileNotFound);
        }
        
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Unauthorized access to mobile API (cookies may be required): {}. Body: {}", username, body);
            return Err(ScraperError::UnauthorizedAccess(body));
        }
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch profile via mobile API, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        // Try to get JSON data from the response
        match response.text().await {
            Ok(text_body) => {
                if text_body.is_empty() {
                    error!("Empty mobile API response body for {}", username);
                    return Err(ScraperError::ParsingError("Empty response body".to_string()));
                }
                
                // Log the response body for debugging
                info!("Mobile API response body: {}", text_body);
                
                // Try to parse the JSON
                match serde_json::from_str::<Value>(&text_body) {
                    Ok(json_data) => {
                        // Log the complete JSON structure if authentication is used
                        if self.config.instagram_cookies.is_some() {
                            debug!("Mobile API authenticated response structure: {}", 
                                  serde_json::to_string_pretty(&json_data)
                                  .unwrap_or_else(|_| "Failed to format JSON".to_string()));
                        }
                        
                        if let Some(data) = json_data.get("data").and_then(|d| d.get("user")) {
                            // Check if the profile is private
                            if let Some(is_private) = data.get("is_private").and_then(|p| p.as_bool()) {
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
                               self.config.instagram_cookies.is_some()
                            {
                                // We can try to fetch additional posts if we have auth cookies
                                info!("Initial fetch returned no posts but post count > 0. Trying to fetch posts via API...");
                                
                                // Get the user ID for pagination
                                if let Some(user_id) = data.get("id").and_then(|id| id.as_str()) {
                                    match self.fetch_user_posts_paged(user_id, username, proxy_url).await {
                                        Ok(posts) => {
                                            user_data.posts = Some(posts);
                                            user_data.posts_limited = true;
                                        },
                                        Err(e) => {
                                            warn!("Failed to fetch additional posts: {}", e);
                                        }
                                    }
                                }
                            }
                            
                            return Ok(user_data);
                        }
                    },
                    Err(e) => {
                        if text_body.trim().is_empty() {
                            error!("Failed to parse mobile API JSON response: {}. Error: {}, Response body is empty", username, e);
                        } else if text_body.len() < 100 {
                            // If response is very short, log the full content
                            error!("Failed to parse mobile API JSON response: {}. Error: {}, Short response body: {}", username, e, text_body);
                        } else {
                            // For longer responses, log a preview
                            let preview = if text_body.len() > 500 { &text_body[0..500] } else { &text_body };
                            error!("Failed to parse mobile API JSON response: {}. Error: {}, Response body preview: {}...", username, e, preview);
                        }
                    }
                }
            },
            Err(e) => {
                error!("Failed to get mobile API response body: {}. Error: {}", username, e);
            }
        }
        
        Err(ScraperError::ParsingError("Could not extract data from mobile API".to_string()))
    }
    
    async fn try_html_scraping(&self, username: &str) -> Result<InstagramUser, ScraperError> {
        // Try to scrape from the standard HTML page
        let url = format!("https://www.instagram.com/{}/", username);
        
        info!("Trying HTML scraping for {}", username);
        
        if let Some(proxy_manager) = &self.proxy_manager {
            // Try with each proxy until one works or all fail
            let mut last_error = None;
            
            // Get proxy count to know how many to try
            let (available, total) = proxy_manager.get_proxy_count();
            
            // If no proxies are available, return error - don't try without proxy
            if available == 0 {
                if total > 0 {
                    warn!("No proxies available (all marked as unavailable), not falling back to direct connection");
                    return Err(ScraperError::AllProxiesFailed);
                } else {
                    warn!("No proxies configured");
                    return Err(ScraperError::ProxyError("No proxies configured".to_string()));
                }
            }
            
            // Try up to available_proxies number of proxies
            for _ in 0..available {
                if let Some(proxy_url) = proxy_manager.get_random_proxy() {
                    info!("Trying HTML request with proxy: {}", proxy_url);
                    
                    match self.make_html_request(&url, username, Some(&proxy_url)).await {
                        Ok(result) => {
                            return Ok(result);
                        }
                        Err(err) => {
                            // If it's a proxy error, mark this proxy as unavailable
                            if let ScraperError::ProxyError(msg) = &err {
                                warn!("Proxy error: {}, marking proxy as unavailable", msg);
                                proxy_manager.mark_proxy_unavailable(&proxy_url);
                            }
                            last_error = Some(err);
                        }
                    }
                }
            }
            
            // If we reached here, all proxies failed
            if let Some(err) = last_error {
                warn!("All proxies failed for HTML scraping: {}", err);
            }
            return Err(ScraperError::AllProxiesFailed);
        } else {
            // No proxy manager, use the default client
            return self.make_html_request(&url, username, None).await;
        }
    }
    
    async fn make_html_request(&self, url: &str, username: &str, proxy_url: Option<&str>) -> Result<InstagramUser, ScraperError> {
        let client_builder = Client::builder()
            .timeout(Duration::from_secs(self.config.timeout))
            .user_agent(&self.config.user_agent);
            
        // Add proxy if provided
        let client_builder = if let Some(proxy) = proxy_url {
            if let Some(proxy_manager) = &self.proxy_manager {
                // Use the normalized proxy URL with explicit protocol
                let normalized_proxy = proxy_manager.normalize_proxy_url(proxy);
                info!("Using normalized proxy URL: {}", normalized_proxy);
                match Proxy::all(&normalized_proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            } else {
                // Fallback to original behavior if no proxy manager
                match Proxy::all(proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            }
        } else {
            client_builder
        };
        
        let client = match client_builder.build() {
            Ok(client) => client,
            Err(e) => return Err(ScraperError::ProxyError(format!("Failed to build client: {}", e))),
        };
        
        // Build request with appropriate headers for HTML page
        let mut request = client.get(url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.5");
        
        // Add cookies if available
        if let Some(cookies) = &self.config.instagram_cookies {
            info!("Using Instagram cookies for HTML scraping (limited to first page of posts)");
            request = request.header("Cookie", cookies);
        }
        
        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                if let Some(_proxy) = proxy_url {
                    return Err(ScraperError::ProxyError(format!("Proxy request failed: {}", e)));
                }
                return Err(ScraperError::NetworkError(e));
            }
        };
        
        let status = response.status();
        
        // Log headers for debugging
        self.log_response_headers(&response, "HTML");
        
        if status == reqwest::StatusCode::NOT_FOUND {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Profile not found via HTML: {}. Body: {}", username, body);
            return Err(ScraperError::ProfileNotFound);
        }
        
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch profile HTML, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        // Get the HTML content
        match response.text().await {
            Ok(html) => {
                if html.is_empty() {
                    error!("Empty HTML response body for {}", username);
                    return Err(ScraperError::ParsingError("Empty response body".to_string()));
                }
                
                // If response is too short, it might be a captcha or error page
                if html.len() < 1000 {
                    error!("HTML response too short (likely blocked or captcha): {}. Body: {}", username, html);
                    return Err(ScraperError::ParsingError("HTML response too short, likely blocked".to_string()));
                }
                
                // Log the first 500 characters of the HTML for debugging if it's a suspicious response
                if html.len() < 5000 || html.contains("captcha") || html.contains("suspicious") {
                    let preview = if html.len() > 500 { &html[0..500] } else { &html };
                    warn!("Suspicious HTML response for {}, preview: {}...", username, preview);
                }
                
                // Try to extract user data from additional data sources in the HTML
                if let Some(user_data) = self.extract_from_additional_data_sources(&html, username) {
                    return Ok(user_data);
                }
                
                // Other extraction attempts...
                // ... existing code ...
            },
            Err(e) => {
                error!("Failed to get HTML response body: {}. Error: {}", username, e);
                return Err(ScraperError::NetworkError(e));
            }
        }
        
        error!("Failed to extract data from HTML sources for {}", username);
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
                    .and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
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
                .and_then(|ts| Utc.timestamp_opt(ts, 0).single());
            
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
    async fn fetch_user_posts_paged(&self, user_id: &str, _username: &str, proxy_url: Option<&str>) -> Result<Vec<InstagramPost>, ScraperError> {
        // Make a request to get the first page of posts
        let url = format!("https://www.instagram.com/graphql/query/?query_hash=8c2a529969ee035a5063f2fc8602a0fd&variables=%7B%22id%22%3A%22{}%22%2C%22first%22%3A12%7D", user_id);
        
        let client_builder = Client::builder()
            .timeout(Duration::from_secs(self.config.timeout))
            .user_agent(&self.config.user_agent);
        
        // Add proxy if provided
        let client_builder = if let Some(proxy) = proxy_url {
            if let Some(proxy_manager) = &self.proxy_manager {
                // Use the normalized proxy URL with explicit protocol
                let normalized_proxy = proxy_manager.normalize_proxy_url(proxy);
                info!("Using normalized proxy URL: {}", normalized_proxy);
                match Proxy::all(&normalized_proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            } else {
                // Fallback to original behavior if no proxy manager
                match Proxy::all(proxy) {
                    Ok(proxy) => client_builder.proxy(proxy),
                    Err(e) => return Err(ScraperError::ProxyError(format!("Failed to create proxy: {}", e))),
                }
            }
        } else {
            client_builder
        };
        
        let client = match client_builder.build() {
            Ok(client) => client,
            Err(e) => return Err(ScraperError::ProxyError(format!("Failed to build client: {}", e))),
        };
        
        let response = match client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => return Err(ScraperError::NetworkError(e)),
        };
        
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string());
            error!("Failed to fetch posts, status: {}. Body: {}", status, body);
            return Err(ScraperError::ParsingError(format!("HTTP error status: {}", status)));
        }
        
        let json_data = response.json::<Value>().await?;
        
        if let Some(data) = json_data.get("data").and_then(|d| d.get("user")) {
            // Fix the Option handling instead of using ? operator
            let timeline = match data.get("edge_owner_to_timeline_media") {
                Some(t) => t,
                None => return Err(ScraperError::ParsingError("Missing edge_owner_to_timeline_media in response".to_string())),
            };
            
            let edges = match timeline.get("edges") {
                Some(e) => e,
                None => return Err(ScraperError::ParsingError("Missing edges in timeline media".to_string())),
            };
            
            let edges_array = match edges.as_array() {
                Some(arr) => arr,
                None => return Err(ScraperError::ParsingError("Edges is not an array".to_string())),
            };
            
            match self.extract_posts_from_items(edges_array) {
                Some(posts) => Ok(posts),
                None => Err(ScraperError::ParsingError("Failed to extract posts from edges".to_string())),
            }
        } else {
            error!("Failed to extract posts from response");
            Err(ScraperError::ParsingError("Failed to extract posts from response".to_string()))
        }
    }
    
    fn log_response_headers(&self, response: &reqwest::Response, endpoint_type: &str) {
        let headers = response.headers();
        let status = response.status();
        
        let mut header_log = format!("Response headers from {} (status {}): \n", endpoint_type, status);
        for (name, value) in headers.iter() {
            if let Ok(value_str) = value.to_str() {
                header_log.push_str(&format!("  {}: {}\n", name, value_str));
            }
        }
        
        // Only log headers if status isn't successful or has specific headers that indicate blocking
        if !status.is_success() || 
           headers.contains_key("x-ratelimit-remaining") || 
           headers.contains_key("x-instagram-error") || 
           headers.contains_key("x-fb-debug") {
            info!("{}", header_log);
        }
    }
} 