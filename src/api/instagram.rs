use rocket::serde::json::Json;
use rocket::State;
use rocket::http::ContentType;
use rocket::{request::Request, response::{self, Response, Responder}};
use std::io::Cursor;

use crate::models::instagram::{InstagramUserResponse, InstagramPostsResponse, InstagramReelsResponse};
use crate::scrapers::instagram::{InstagramScraper, ScraperError};
use crate::cache::{InstagramCache, ImageCache};
use crate::config::AppConfig;
use crate::images::ImageProxy;
use crate::api::ApiError;

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

// Responder for image data
pub struct ImageResponse {
    pub data: Vec<u8>,
    pub content_type: String,
}

impl<'r> Responder<'r, 'static> for ImageResponse {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        // Convert content type string to ContentType
        let content_type = match self.content_type.as_str() {
            "image/jpeg" => ContentType::JPEG,
            "image/png" => ContentType::PNG,
            "image/gif" => ContentType::GIF,
            "image/webp" => ContentType::new("image", "webp"),
            "image/bmp" => ContentType::new("image", "bmp"),
            "image/tiff" => ContentType::new("image", "tiff"),
            "image/x-icon" => ContentType::new("image", "x-icon"),
            _ => ContentType::JPEG, // Default if unknown
        };
        
        Response::build()
            .header(content_type)
            .sized_body(None, Cursor::new(self.data))
            .ok()
    }
}

#[get("/image?<url>")]
pub async fn proxy_image(
    url: &str,
    image_cache: &State<ImageCache>,
    _config: &State<AppConfig>,
    image_proxy: &State<ImageProxy>,
) -> Result<ImageResponse, ApiError> {
    // Check cache first
    if let Some((image_data, content_type)) = image_cache.get_image(url) {
        log::info!("Image found in cache: {}", url);
        return Ok(ImageResponse {
            data: image_data,
            content_type,
        });
    }

    log::info!("Image not found in cache: {}", url);
    
    match image_proxy.fetch_image(url).await {
        Ok((image_data, content_type)) => {
            // Store in cache
            image_cache.store_image(url, image_data.clone(), content_type.clone());
            Ok(ImageResponse {
                data: image_data,
                content_type,
            })
        },
        Err(err) => Err(err.into()),
    }
} 