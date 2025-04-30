# Scrapn - Instagram Scraping API

Scrapn is a REST API built with Rust and Rocket that provides access to Instagram data via scraping.

## Features

- Instagram profile scraping
- JSON REST API endpoints
- Configurable in-memory caching
- Support for posts and reels
- Instagram username whitelist for restricted access

## API Endpoints

### Instagram

- `GET /instagram/<username>` - Get full profile data for an Instagram user
- `GET /instagram/<username>/posts` - Get only posts for an Instagram user
- `GET /instagram/<username>/reels` - Get only reels for an Instagram user

## Response Format

All endpoints return JSON with the following structure:

```json
{
  "data": {...},         // The requested data
  "from_cache": true,    // Whether the data was served from cache
  "cache_age": 3600      // Age of the cached data in seconds (null if not from cache)
}
```

## Configuration

Configuration is stored in `App.toml`:

```toml
[default]
port = 8000
address = "0.0.0.0"  # Use 0.0.0.0 to allow external connections
# Cache duration in days
instagram_cache_duration = 1
# Scraping timeout in seconds
timeout = 30
user_agent = "..."
```

### Environment Variables

- `INSTAGRAM_USERNAME_WHITELIST` - Optional comma-separated list of Instagram usernames that are allowed to be scraped. If set, only these usernames will be accessible through the API.
- `INSTAGRAM_COOKIES` - Optional Instagram session cookies for authenticated requests. This helps bypass rate limits and access restricted content.

Example:
```
INSTAGRAM_USERNAME_WHITELIST=username1,username2,username3
INSTAGRAM_COOKIES=sessionid=YOUR_SESSION_ID; ds_user_id=YOUR_USER_ID; csrftoken=YOUR_CSRF_TOKEN
```

#### Obtaining Instagram Cookies

To get your Instagram cookies:
1. Log in to Instagram in your browser
2. Open browser developer tools (F12 or right-click > Inspect)
3. Go to the Application/Storage tab
4. Find Cookies > www.instagram.com
5. Copy the values of `sessionid`, `ds_user_id`, and other cookies
6. Format as: `sessionid=ABC123; ds_user_id=123456; csrftoken=XYZ789`

**Note:** Cookies typically expire after a few weeks or if Instagram detects unusual activity. You may need to refresh them periodically.

## Building and Running

### Native Build

```
cargo build --release
./target/release/scrapn
```

The server will start on the configured address and port.

### Docker

The application can be run using Docker:

```
# Build the Docker image
docker build -t scrapn .

# Run the container
docker run -p 8000:8000 scrapn
```

Or using Docker Compose:

```
# Start the service
docker-compose up -d

# View logs
docker-compose logs -f
```

The server will start on port 8000, accessible at http://localhost:8000.

## Warning

Web scraping may violate Instagram's Terms of Service. Use responsibly and at your own risk.