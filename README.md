# Scrapn - Instagram Scraping API

Scrapn is a REST API built with Rust and Rocket that provides access to Instagram data via scraping.

## Features

- Instagram profile scraping
- JSON REST API endpoints
- Configurable in-memory caching
- Support for posts and reels
- Instagram username whitelist for restricted access
- Proxy rotation to prevent IP blocking

## API Endpoints

### Instagram

- `GET /instagram/<username>` - Get full profile data for an Instagram user
- `GET /instagram/<username>/posts` - Get only posts for an Instagram user
- `GET /instagram/<username>/reels` - Get only reels for an Instagram user
- `GET /instagram/image?url=<encoded_url>` - Proxy for Instagram CDN images with permanent caching

## Response Format

All data endpoints return JSON with the following structure:

```json
{
  "data": {...},         // The requested data
  "from_cache": true,    // Whether the data was served from cache
  "cache_age": 3600      // Age of the cached data in seconds (null if not from cache)
}
```

The image proxy endpoint returns the image data directly with the appropriate content type header.

### Using the Image Proxy

To use the image proxy, you need to URL-encode the Instagram CDN URL:

```
/instagram/image?url=https%3A%2F%2Fscontent-iad3-2.cdninstagram.com%2Fv%2Ft51.2885-15%2F123456_789012345678901_1234567890123456789_n.jpg%3F...
```

#### URL Encoding Examples

JavaScript:
```javascript
const instagramUrl = "https://scontent-lga3-3.cdninstagram.com/v/t51.2885-15/123456.jpg?param1=value1&param2=value2";
const encodedUrl = encodeURIComponent(instagramUrl);
const proxyUrl = `/instagram/image?url=${encodedUrl}`;

// Use in HTML
const imgElement = document.createElement('img');
imgElement.src = proxyUrl;
document.body.appendChild(imgElement);
```

Python:
```python
import urllib.parse

instagram_url = "https://scontent-lga3-3.cdninstagram.com/v/t51.2885-15/123456.jpg?param1=value1&param2=value2"
encoded_url = urllib.parse.quote(instagram_url)
proxy_url = f"/instagram/image?url={encoded_url}"
```

This helps circumvent direct hotlinking restrictions and provides permanent caching of images. Benefits include:

- Avoids browser-side CORS issues
- Adds proper caching headers
- Uses optimized request headers to bypass CDN restrictions
- Provides consistent access to Instagram images even if URLs change
- Reduces bandwidth usage through permanent caching

#### Content Type Detection

The image proxy automatically detects the correct content type (MIME type) for images using:

1. Response headers from Instagram CDN
2. File signature analysis (magic numbers) as fallback

Supported formats include:
- JPEG (`image/jpeg`)
- PNG (`image/png`)
- GIF (`image/gif`)
- WebP (`image/webp`)
- BMP (`image/bmp`)
- TIFF (`image/tiff`)
- ICO (`image/x-icon`)

#### Image Caching

Images are cached permanently in memory to:
- Reduce bandwidth usage
- Decrease load times for frequently accessed images
- Limit requests to Instagram's CDN
- Provide image availability even if the source is temporarily unavailable

**Note:** Since caching is in-memory, images are lost if the server restarts.

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

# Proxy configuration (optional)
[default.proxy]
# Comma-separated list of proxy URLs
proxies = [
  "http://user:pass@host:port",
  "socks5://user:pass@host:port"
]
# Time in hours to mark a proxy as unavailable after failure
unavailable_time = 4
```

### Environment Variables

- `INSTAGRAM_USERNAME_WHITELIST` - Optional comma-separated list of Instagram usernames that are allowed to be scraped. If set, only these usernames will be accessible through the API.
- `INSTAGRAM_COOKIES` - Optional Instagram session cookies for authenticated requests. This helps bypass rate limits and access restricted content.
- `INSTAGRAM_PROXIES` - Optional comma-separated list of proxy URLs. This helps prevent IP blocking by rotating between multiple proxies.

Example:
```
INSTAGRAM_USERNAME_WHITELIST=username1,username2,username3
INSTAGRAM_COOKIES=sessionid=YOUR_SESSION_ID; ds_user_id=YOUR_USER_ID; csrftoken=YOUR_CSRF_TOKEN
INSTAGRAM_PROXIES=http://user:pass@host1:port1,http://user:pass@host2:port2,socks5://user:pass@host3:port3
```

#### Supported Proxy Protocols

Scrapn supports the following proxy protocols:
- HTTP
- HTTPS
- SOCKS4
- SOCKS5

Each proxy URL should include the protocol, authentication (if required), host, and port.

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

Web scraping may violate Instagram's Terms of Service. Use responsibly and at your own risk. Instagram may block requests from known proxy IPs, so using residential proxies is recommended for better results.