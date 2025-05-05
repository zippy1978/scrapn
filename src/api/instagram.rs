use rocket::serde::json::Json;
use rocket::State;
use rocket::http::Status;
use rocket::{request::Request, response::{self, Response, Responder}};
use serde_json::json;

use crate::models::instagram::{InstagramUserResponse, InstagramPostsResponse, InstagramReelsResponse};
use crate::scrapers::instagram::{InstagramScraper, ScraperError};
use crate::cache::InstagramCache;
use crate::config::AppConfig;

#[derive(Debug)]
pub enum ApiError {
    ScraperError(ScraperError),
    SerializationError(String),
}

impl From<ScraperError> for ApiError {
    fn from(error: ScraperError) -> Self {
        ApiError::ScraperError(error)
    }
}

impl<'r> rocket::response::Responder<'r, 'static> for ApiError {
    fn respond_to(self, _: &'r rocket::Request<'_>) -> rocket::response::Result<'static> {
        match self {
            ApiError::ScraperError(ScraperError::ProfileNotFound) => {
                rocket::Response::build()
                    .status(Status::NotFound)
                    .sized_body(None, std::io::Cursor::new("Profile not found"))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::PrivateProfile) => {
                let body = json!({
                    "error": "Profile is private",
                    "message": "The requested profile is private and cannot be accessed"
                }).to_string();
                
                rocket::Response::build()
                    .status(Status::Forbidden)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::RateLimited) => {
                let body = json!({
                    "error": "Rate limited",
                    "message": "Too many requests, please try again later"
                }).to_string();
                
                rocket::Response::build()
                    .status(Status::TooManyRequests)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::UnauthorizedAccess(message)) => {
                let body = json!({
                    "error": "Unauthorized",
                    "message": message
                }).to_string();
                
                rocket::Response::build()
                    .status(Status::Unauthorized)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::ProxyError(error)) => {
                let body = json!({
                    "error": "Proxy error",
                    "message": error
                }).to_string();
                
                rocket::Response::build()
                    .status(Status::BadGateway)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::AllProxiesFailed) => {
                let body = json!({
                    "error": "All proxies failed",
                    "message": "All configured proxies failed to connect"
                }).to_string();
                
                rocket::Response::build()
                    .status(Status::ServiceUnavailable)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::NetworkError(error)) => {
                let body = json!({
                    "error": "Network error",
                    "message": error.to_string()
                }).to_string();
                
                rocket::Response::build()
                    .status(Status::ServiceUnavailable)
                    .sized_body(None, std::io::Cursor::new(body))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::ParsingError(e)) => {
                rocket::Response::build()
                    .status(Status::InternalServerError)
                    .sized_body(None, std::io::Cursor::new(format!("Error parsing Instagram page: {}", e)))
                    .ok()
            },
            ApiError::SerializationError(e) => {
                rocket::Response::build()
                    .status(Status::InternalServerError)
                    .sized_body(None, std::io::Cursor::new(e))
                    .ok()
            }
        }
    }
}

#[get("/<username>")]
pub async fn get_user(
    username: &str,
    scraper: &State<InstagramScraper>,
    cache: &State<InstagramCache>,
    config: &State<AppConfig>,
) -> Result<Json<InstagramUserResponse>, ApiError> {
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Check cache first (non-expired data)
    if let Some((user, age)) = cache.get_user(username) {
        return Ok(Json(InstagramUserResponse {
            data: user,
            from_cache: true,
            cache_age: Some(age),
        }));
    }
    
    // Try to scrape fresh data
    match scraper.scrape_user(username).await {
        Ok(user) => {
            // Successfully retrieved fresh data, store in cache
            cache.store_user(user.clone());
            
            Ok(Json(InstagramUserResponse {
                data: user,
                from_cache: false,
                cache_age: None,
            }))
        },
        Err(err) => {
            // Scraping failed, try to use expired cache data as fallback
            if let Some((user, age)) = cache.get_user_even_expired(username) {
                // Log that we're using expired cache as fallback
                log::warn!("Using expired cache for {} as fallback due to scraping error: {:?}", username, err);
                
                Ok(Json(InstagramUserResponse {
                    data: user,
                    from_cache: true,
                    cache_age: Some(age),
                }))
            } else {
                // No cache data available, return the error
                Err(err.into())
            }
        }
    }
}

#[get("/<username>/posts")]
pub async fn get_posts(
    username: &str,
    scraper: &State<InstagramScraper>,
    cache: &State<InstagramCache>,
    config: &State<AppConfig>,
) -> Result<Json<InstagramPostsResponse>, ApiError> {
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Check cache first (non-expired data)
    if let Some((posts, age)) = cache.get_posts(username) {
        return Ok(Json(InstagramPostsResponse {
            data: posts,
            from_cache: true,
            cache_age: Some(age),
        }));
    }
    
    // Try to scrape fresh data
    match scraper.scrape_user(username).await {
        Ok(user) => {
            // Successfully retrieved fresh data, store in cache
            cache.store_user(user.clone());
            
            // Return posts
            let posts = user.posts.unwrap_or_default();
            
            Ok(Json(InstagramPostsResponse {
                data: posts,
                from_cache: false,
                cache_age: None,
            }))
        },
        Err(err) => {
            // Scraping failed, try to use expired cache data as fallback
            if let Some((posts, age)) = cache.get_posts_even_expired(username) {
                // Log that we're using expired cache as fallback
                log::warn!("Using expired cache for {}/posts as fallback due to scraping error: {:?}", username, err);
                
                Ok(Json(InstagramPostsResponse {
                    data: posts,
                    from_cache: true,
                    cache_age: Some(age),
                }))
            } else {
                // No cache data available, return the error
                Err(err.into())
            }
        }
    }
}

#[get("/<username>/reels")]
pub async fn get_reels(
    username: &str,
    scraper: &State<InstagramScraper>,
    cache: &State<InstagramCache>,
    config: &State<AppConfig>,
) -> Result<Json<InstagramReelsResponse>, ApiError> {
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Check cache first (non-expired data)
    if let Some((reels, age)) = cache.get_reels(username) {
        return Ok(Json(InstagramReelsResponse {
            data: reels,
            from_cache: true,
            cache_age: Some(age),
        }));
    }
    
    // Try to scrape fresh data
    match scraper.scrape_user(username).await {
        Ok(user) => {
            // Successfully retrieved fresh data, store in cache
            cache.store_user(user.clone());
            
            // Return reels
            let reels = user.reels.unwrap_or_default();
            
            Ok(Json(InstagramReelsResponse {
                data: reels,
                from_cache: false,
                cache_age: None,
            }))
        },
        Err(err) => {
            // Scraping failed, try to use expired cache data as fallback
            if let Some((reels, age)) = cache.get_reels_even_expired(username) {
                // Log that we're using expired cache as fallback
                log::warn!("Using expired cache for {}/reels as fallback due to scraping error: {:?}", username, err);
                
                Ok(Json(InstagramReelsResponse {
                    data: reels,
                    from_cache: true,
                    cache_age: Some(age),
                }))
            } else {
                // No cache data available, return the error
                Err(err.into())
            }
        }
    }
} 