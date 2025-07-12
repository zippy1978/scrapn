use rocket::State;
use rocket::http::ContentType;
use rocket::{request::Request, response::{self, Response, Responder}};
use std::io::Cursor;
use md5;
use rocket::http::Header;
use serde;

use crate::models::instagram::{InstagramUserResponse, InstagramPostsResponse, InstagramReelsResponse};
use crate::scrapers::instagram::{InstagramScraper, ScraperError};
use crate::cache::{InstagramCache, ImageCache};
use crate::config::AppConfig;
use crate::images::{ImageProxy, ImageConversionParams};
use crate::api::ApiError;

#[get("/<username>")]
pub async fn get_user(
    username: &str,
    scraper: &State<InstagramScraper>,
    cache: &State<InstagramCache>,
    config: &State<AppConfig>,
) -> Result<JsonWithCache<InstagramUserResponse>, ApiError> {
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Check cache first (non-expired data)
    if let Some((user, age)) = cache.get_user(username) {
        return Ok(JsonWithCache {
            inner: InstagramUserResponse {
                data: user,
                from_cache: true,
                cache_age: Some(age),
            },
            from_cache: true,
            cache_age: Some(age),
            cache_duration: cache.cache_duration.as_secs(),
        });
    }
    
    // Try to scrape fresh data with retry logic
    match scraper.scrape_user_with_retry(username).await {
        Ok(user) => {
            // Successfully retrieved fresh data, store in cache
            cache.store_user(user.clone());
            
            Ok(JsonWithCache {
                inner: InstagramUserResponse {
                    data: user,
                    from_cache: false,
                    cache_age: None,
                },
                from_cache: false,
                cache_age: None,
                cache_duration: cache.cache_duration.as_secs(),
            })
        },
        Err(err) => {
            // Scraping failed, try to use expired cache data as fallback
            if let Some((user, age)) = cache.get_user_even_expired(username) {
                // Log that we're using expired cache as fallback
                log::warn!("Using expired cache for {} as fallback due to scraping error: {:?}", username, err);
                
                Ok(JsonWithCache {
                    inner: InstagramUserResponse {
                        data: user,
                        from_cache: true,
                        cache_age: Some(age),
                    },
                    from_cache: true,
                    cache_age: Some(age),
                    cache_duration: cache.cache_duration.as_secs(),
                })
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
) -> Result<JsonWithCache<InstagramPostsResponse>, ApiError> {
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Check cache first (non-expired data)
    if let Some((posts, age)) = cache.get_posts(username) {
        return Ok(JsonWithCache {
            inner: InstagramPostsResponse {
                data: posts,
                from_cache: true,
                cache_age: Some(age),
            },
            from_cache: true,
            cache_age: Some(age),
            cache_duration: cache.cache_duration.as_secs(),
        });
    }
    
    // Try to scrape fresh data with retry logic
    match scraper.scrape_user_with_retry(username).await {
        Ok(user) => {
            // Successfully retrieved fresh data, store in cache
            cache.store_user(user.clone());
            
            // Return posts
            let posts = user.posts.unwrap_or_default();
            
            Ok(JsonWithCache {
                inner: InstagramPostsResponse {
                    data: posts,
                    from_cache: false,
                    cache_age: None,
                },
                from_cache: false,
                cache_age: None,
                cache_duration: cache.cache_duration.as_secs(),
            })
        },
        Err(err) => {
            // Scraping failed, try to use expired cache data as fallback
            if let Some((posts, age)) = cache.get_posts_even_expired(username) {
                // Log that we're using expired cache as fallback
                log::warn!("Using expired cache for {}/posts as fallback due to scraping error: {:?}", username, err);
                
                Ok(JsonWithCache {
                    inner: InstagramPostsResponse {
                        data: posts,
                        from_cache: true,
                        cache_age: Some(age),
                    },
                    from_cache: true,
                    cache_age: Some(age),
                    cache_duration: cache.cache_duration.as_secs(),
                })
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
) -> Result<JsonWithCache<InstagramReelsResponse>, ApiError> {
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Check cache first (non-expired data)
    if let Some((reels, age)) = cache.get_reels(username) {
        return Ok(JsonWithCache {
            inner: InstagramReelsResponse {
                data: reels,
                from_cache: true,
                cache_age: Some(age),
            },
            from_cache: true,
            cache_age: Some(age),
            cache_duration: cache.cache_duration.as_secs(),
        });
    }
    
    // Try to scrape fresh data with retry logic
    match scraper.scrape_user_with_retry(username).await {
        Ok(user) => {
            // Successfully retrieved fresh data, store in cache
            cache.store_user(user.clone());
            
            // Return reels
            let reels = user.reels.unwrap_or_default();
            
            Ok(JsonWithCache {
                inner: InstagramReelsResponse {
                    data: reels,
                    from_cache: false,
                    cache_age: None,
                },
                from_cache: false,
                cache_age: None,
                cache_duration: cache.cache_duration.as_secs(),
            })
        },
        Err(err) => {
            // Scraping failed, try to use expired cache data as fallback
            if let Some((reels, age)) = cache.get_reels_even_expired(username) {
                // Log that we're using expired cache as fallback
                log::warn!("Using expired cache for {}/reels as fallback due to scraping error: {:?}", username, err);
                
                Ok(JsonWithCache {
                    inner: InstagramReelsResponse {
                        data: reels,
                        from_cache: true,
                        cache_age: Some(age),
                    },
                    from_cache: true,
                    cache_age: Some(age),
                    cache_duration: cache.cache_duration.as_secs(),
                })
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
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
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
        let etag = format!("\"{:x}\"", md5::compute(&self.data));
        // Check If-None-Match header
        if let Some(if_none_match) = req.headers().get_one("If-None-Match") {
            if if_none_match == etag {
                // ETag matches, return 304 Not Modified
                return Response::build()
                    .status(rocket::http::Status::NotModified)
                    .header(Header::new("ETag", etag))
                    .header(Header::new("Cache-Control", "public, max-age=86400"))
                    .ok();
            }
        }
        Response::build()
            .header(content_type)
            .header(Header::new("Cache-Control", "public, max-age=86400"))
            .header(Header::new("ETag", etag))
            .sized_body(None, Cursor::new(self.data))
            .ok()
    }
}

#[derive(FromForm)]
pub struct ImageProxyQuery {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<String>,
    pub quality: Option<u8>,
    pub fit: Option<String>,
    pub focus: Option<String>,
}

impl ImageProxyQuery {
    fn to_conversion_params(&self) -> Result<ImageConversionParams, ApiError> {
        let format = if let Some(ref fmt) = self.format {
            Some(match fmt.as_str() {
                "webp" => crate::images::ImageConversionFormat::Webp,
                "jpg" | "jpeg" => crate::images::ImageConversionFormat::Jpg,
                "png" => crate::images::ImageConversionFormat::Png,
                "gif" => crate::images::ImageConversionFormat::Gif,
                "avif" => crate::images::ImageConversionFormat::Avif,
                _ => return Err(ApiError::ScraperError(crate::scrapers::instagram::ScraperError::ParsingError(
                    format!("Unsupported format: {}", fmt)
                ))),
            })
        } else {
            None
        };
        
        let fit = if let Some(ref fit_str) = self.fit {
            Some(match fit_str.as_str() {
                "pad" => crate::images::ImageFit::Pad,
                "fill" => crate::images::ImageFit::Fill,
                "scale" => crate::images::ImageFit::Scale,
                "crop" => crate::images::ImageFit::Crop,
                "thumb" => crate::images::ImageFit::Thumb,
                _ => return Err(ApiError::ScraperError(crate::scrapers::instagram::ScraperError::ParsingError(
                    format!("Unsupported fit: {}", fit_str)
                ))),
            })
        } else {
            None
        };
        
        let focus = if let Some(ref focus_str) = self.focus {
            Some(match focus_str.as_str() {
                "center" => crate::images::ImageFocus::Center,
                "top" => crate::images::ImageFocus::Top,
                "right" => crate::images::ImageFocus::Right,
                "left" => crate::images::ImageFocus::Left,
                "bottom" => crate::images::ImageFocus::Bottom,
                "top_right" => crate::images::ImageFocus::TopRight,
                "top_left" => crate::images::ImageFocus::TopLeft,
                "bottom_right" => crate::images::ImageFocus::BottomRight,
                "bottom_left" => crate::images::ImageFocus::BottomLeft,
                "face" => crate::images::ImageFocus::Face,
                "faces" => crate::images::ImageFocus::Faces,
                _ => return Err(ApiError::ScraperError(crate::scrapers::instagram::ScraperError::ParsingError(
                    format!("Unsupported focus: {}", focus_str)
                ))),
            })
        } else {
            None
        };
        
        Ok(ImageConversionParams {
            width: self.width,
            height: self.height,
            format,
            quality: self.quality,
            fit,
            focus,
        })
    }
}

#[get("/<username>/image?<query..>")]
pub async fn proxy_image(
    username: &str,
    query: ImageProxyQuery,
    image_cache: &State<ImageCache>,
    config: &State<AppConfig>,
    image_proxy: &State<ImageProxy>,
    scraper: &State<InstagramScraper>,
    cache: &State<InstagramCache>,
) -> Result<ImageResponse, ApiError> {
    log::debug!("Proxying image for user '{}', URL: {}", username, query.url);
    
    // Convert query parameters to conversion params
    let conversion_params = query.to_conversion_params()?;
    
    // Whitelist check
    if let Some(whitelist) = &config.instagram_username_whitelist {
        if !whitelist.contains(&username.to_string()) {
            log::warn!("Username '{}' not in whitelist", username);
            return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(format!("Username '{}' not allowed", username))));
        }
    }
    
    // Verify URL belongs to the user by checking against cached user data
    let user_data = match cache.get_user_even_expired(username) {
        Some((user, _)) => {
            log::debug!("Found cached user data for '{}'", username);
            user
        },
        None => {
            // Try to fetch user data if not in cache
            log::debug!("No cached data for '{}', fetching fresh data", username);
            match scraper.scrape_user(username).await {
                Ok(user) => {
                    cache.store_user(user.clone());
                    user
                },
                Err(err) => {
                    log::error!("Failed to fetch user data for '{}': {:?}", username, err);
                    return Err(ApiError::ScraperError(err))
                }
            }
        }
    };
    
    // Check if URL belongs to user's content using the new method
    if !user_data.is_content_url(&query.url) {
        log::warn!("URL '{}' does not belong to user '{}'", query.url, username);
        log::debug!("User has {} posts and {} reels", 
            user_data.posts.as_ref().map_or(0, |p| p.len()),
            user_data.reels.as_ref().map_or(0, |r| r.len()));
        
        return Err(ApiError::ScraperError(ScraperError::UnauthorizedAccess(
            format!("URL '{}' does not belong to user '{}'", query.url, username)
        )));
    }
    
    log::debug!("URL validation passed for '{}'", query.url);
    
    // Check cache first
    if let Some((image_data, content_type)) = image_cache.get_image(&query.url, &conversion_params) {
        log::info!("Processed image found in cache: {} with params: {:?}", query.url, conversion_params);
        return Ok(ImageResponse {
            data: image_data,
            content_type,
        });
    }

    log::info!("Processed image not found in cache: {} with params: {:?}", query.url, conversion_params);
    
    match image_proxy.fetch_and_convert_image(&query.url, &conversion_params).await {
        Ok((image_data, content_type)) => {
            // Store in cache
            image_cache.store_image(&query.url, &conversion_params, image_data.clone(), content_type.clone());
            Ok(ImageResponse {
                data: image_data,
                content_type,
            })
        },
        Err(err) => Err(err.into()),
    }
}

pub struct JsonWithCache<T> {
    pub inner: T,
    pub from_cache: bool,
    pub cache_age: Option<u64>,
    pub cache_duration: u64,
}

impl<'r, T: serde::Serialize> Responder<'r, 'static> for JsonWithCache<T> {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut response = Response::build();
        response.header(ContentType::JSON);
        // Set cache headers
        if self.from_cache {
            // If from cache, set max-age to remaining cache duration
            let max_age = self.cache_age.map(|age| self.cache_duration.saturating_sub(age)).unwrap_or(self.cache_duration);
            response.header(Header::new("Cache-Control", format!("public, max-age={}", max_age)));
        } else {
            // If fresh, set max-age to full cache duration
            response.header(Header::new("Cache-Control", format!("public, max-age={}", self.cache_duration)));
        }
        response.sized_body(None, Cursor::new(serde_json::to_vec(&self.inner).unwrap()));
        response.ok()
    }
} 