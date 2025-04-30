use rocket::serde::json::Json;
use rocket::State;
use rocket::http::Status;

use crate::models::instagram::{InstagramUserResponse, InstagramPostsResponse, InstagramReelsResponse};
use crate::scrapers::instagram::{InstagramScraper, ScraperError};
use crate::cache::InstagramCache;
use crate::config::AppConfig;

#[derive(Debug)]
#[allow(dead_code)]
pub enum ApiError {
    ScraperError(ScraperError),
    InternalError(String),
    Forbidden(String),
}

impl From<ScraperError> for ApiError {
    fn from(error: ScraperError) -> Self {
        ApiError::ScraperError(error)
    }
}

impl<'r> rocket::response::Responder<'r, 'static> for ApiError {
    fn respond_to(self, _request: &'r rocket::Request<'_>) -> rocket::response::Result<'static> {
        match self {
            ApiError::ScraperError(ScraperError::ProfileNotFound) => {
                rocket::Response::build()
                    .status(Status::NotFound)
                    .sized_body(None, std::io::Cursor::new("Profile not found"))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::RateLimited) => {
                rocket::Response::build()
                    .status(Status::TooManyRequests)
                    .sized_body(None, std::io::Cursor::new("Rate limited by Instagram"))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::PrivateProfile) => {
                rocket::Response::build()
                    .status(Status::Forbidden)
                    .sized_body(None, std::io::Cursor::new("Private profile"))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::NetworkError(e)) => {
                rocket::Response::build()
                    .status(Status::ServiceUnavailable)
                    .sized_body(None, std::io::Cursor::new(format!("Network error: {}", e)))
                    .ok()
            },
            ApiError::ScraperError(ScraperError::ParsingError(e)) => {
                rocket::Response::build()
                    .status(Status::InternalServerError)
                    .sized_body(None, std::io::Cursor::new(format!("Error parsing Instagram page: {}", e)))
                    .ok()
            },
            ApiError::InternalError(e) => {
                rocket::Response::build()
                    .status(Status::InternalServerError)
                    .sized_body(None, std::io::Cursor::new(format!("Internal error: {}", e)))
                    .ok()
            },
            ApiError::Forbidden(e) => {
                rocket::Response::build()
                    .status(Status::Forbidden)
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
            return Err(ApiError::Forbidden(format!("Username '{}' not allowed", username)));
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
            return Err(ApiError::Forbidden(format!("Username '{}' not allowed", username)));
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
            return Err(ApiError::Forbidden(format!("Username '{}' not allowed", username)));
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