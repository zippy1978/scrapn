#[macro_use]
extern crate rocket;

mod api;
mod cache;
mod config;
mod models;
mod scrapers;

use std::env;

use cache::InstagramCache;
use config::AppConfig;
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

    figment = figment.select(Profile::from_env_or("APP_PROFILE", "default"));

    info!("Configuration loaded successfully");

    // App config
    let config = figment.extract::<AppConfig>().unwrap();

    // Initialize logger
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    // Create Instagram scraper
    let instagram_scraper = InstagramScraper::new(config.clone());

    // Create Instagram cache
    let instagram_cache = InstagramCache::new(config.instagram_cache_duration);

    info!(
        "Starting Scrapn API server on {}:{}",
        config.address, config.port
    );

    // Build Rocket instance
    rocket::custom(figment)
        .attach(CORS)
        .manage(instagram_scraper)
        .manage(instagram_cache)
        .manage(config.clone())
        .mount(
            "/instagram",
            routes![
                api::instagram::get_user,
                api::instagram::get_posts,
                api::instagram::get_reels,
            ],
        )
}
