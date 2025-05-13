#[macro_use]
extern crate rocket;

mod api;
mod cache;
mod config;
mod models;
mod proxy;
mod scrapers;
mod images;

use std::env;

use cache::{InstagramCache, ImageCache};
use config::AppConfig;
use proxy::ProxyManager;
use dotenv::dotenv;
use env_logger::Env;
use log::info;
use rocket::{
    figment::{
        providers::{Format, Toml},
        Figment, Profile,
    },
    Config,
};
use scrapers::instagram::InstagramScraper;
use images::ImageProxy;
use scrapn::cors::CORS;

#[launch]
async fn rocket() -> _ {
    dotenv().ok();

    // Load config
    let mut figment = Figment::from(Config::default())
        .merge(Toml::file("App.toml").nested());

    // Merge Instagram username whitelist
    if let Ok(whitelist) = env::var("INSTAGRAM_USERNAME_WHITELIST") {
        figment = figment.merge(("instagram_username_whitelist", whitelist.split(',').map(|s| s.trim().to_string()).collect::<Vec<String>>()));
    }

    // Merge Instagram cookies if available
    if let Ok(cookies) = env::var("INSTAGRAM_COOKIES") {
        figment = figment.merge(("instagram_cookies", cookies));
    }
    
    // Merge proxies if available from environment
    if let Ok(proxies) = env::var("PROXIES") {
        figment = figment.merge(("proxies", proxies.split(',').map(|s| s.trim().to_string()).collect::<Vec<String>>()));
    }

    figment = figment.select(Profile::from_env_or("APP_PROFILE", "default"));

    info!("Configuration loaded successfully");

    // App config
    let config = figment.extract::<AppConfig>().unwrap();

    // Initialize logger
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    
    // Create proxy manager with 4 hour unavailability period
    let proxy_manager = ProxyManager::new(config.proxies.clone(), 4);
    
    // Log proxy information
    if let Some(_proxies) = &config.proxies {
        let (available, total) = proxy_manager.get_proxy_count();
        info!(
            "Proxy rotation enabled with {}/{} available proxies",
            available, total
        );
    } else {
        info!("Proxy rotation disabled - no proxies configured");
    }

    // Create Instagram scraper
    let instagram_scraper = InstagramScraper::new(config.clone(), proxy_manager.clone());

    // Create Instagram cache
    let instagram_cache = InstagramCache::new(config.instagram_cache_duration);
    
    // Create Instagram image cache (cached permanently)
    let instagram_image_cache = ImageCache::new();
    info!("Instagram image proxy cache initialized (permanent storage)");
    
    // Create image proxy
    let image_proxy = ImageProxy::new(
        config.timeout,
    );
    info!("Image proxy initialized");

    info!(
        "Starting Scrapn API server on {}:{}",
        config.address, config.port
    );

    // Build Rocket instance
    rocket::custom(figment)
        .attach(CORS)
        .manage(instagram_scraper)
        .manage(instagram_cache)
        .manage(instagram_image_cache)
        .manage(image_proxy)
        .manage(config.clone())
        .mount(
            "/instagram",
            routes![
                api::instagram::get_user,
                api::instagram::get_posts,
                api::instagram::get_reels,
                api::instagram::proxy_image,
            ],
        )
}
