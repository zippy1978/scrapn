use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ImageProxyError {
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),
    
    #[error("Proxy error: {0}")]
    ProxyError(String),
    
    #[error("Image error: {0}")]
    ImageError(String),
}

pub struct ImageProxy {
    timeout: Duration,
}

impl ImageProxy {
    pub fn new(timeout: u64) -> Self {
        Self {
            timeout: Duration::from_secs(timeout),
        }
    }
    
    // Fetch an image from a URL
    pub async fn fetch_image(&self, url: &str) -> Result<(Vec<u8>, String), ImageProxyError> {
        self.make_request(url, None).await
    }

    // Make actual HTTP request with or without proxy
    async fn make_request(&self, url: &str, proxy_url: Option<&str>) -> Result<(Vec<u8>, String), ImageProxyError> {
        let client_builder = reqwest::Client::builder()
            .timeout(self.timeout);
        
        // Add proxy if provided
        let client_builder = if let Some(proxy) = proxy_url {
            match reqwest::Proxy::all(proxy) {
                Ok(proxy) => client_builder.proxy(proxy),
                Err(e) => return Err(ImageProxyError::ProxyError(format!("Failed to create proxy: {}", e))),
            }
        } else {
            client_builder
        };
        
        let client = match client_builder.build() {
            Ok(client) => client,
            Err(e) => return Err(ImageProxyError::ProxyError(format!("Failed to build client: {}", e))),
        };
        
        // Build request with headers matching browser request
        let request = client.get(url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15")
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "fr-FR,fr;q=0.9")
            .header("Accept-Encoding", "gzip, deflate, br")
            .header("Priority", "u=0, i")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "none");
        
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if !status.is_success() {
                    return Err(ImageProxyError::ImageError(
                        format!("Image request failed with status: {}", status)
                    ));
                }
                
                // Get the content-type from headers or default to octet-stream
                let content_type = response.headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                
                match response.bytes().await {
                    Ok(bytes) => {
                        let image_data = bytes.to_vec();
                        
                        // If content type is missing or generic, try to detect from image data
                        let content_type = if content_type == "application/octet-stream" || content_type.is_empty() {
                            self.detect_image_type(&image_data)
                        } else {
                            content_type
                        };
                        
                        Ok((image_data, content_type))
                    },
                    Err(e) => Err(ImageProxyError::NetworkError(e)),
                }
            },
            Err(e) => Err(ImageProxyError::NetworkError(e)),
        }
    }

    // Function to detect image type from the image data
    fn detect_image_type(&self, data: &[u8]) -> String {
        if data.len() < 8 {
            return "application/octet-stream".to_string();
        }
        
        // Check file signatures (magic numbers)
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            // JPEG signature
            return "image/jpeg".to_string();
        } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            // PNG signature
            return "image/png".to_string();
        } else if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
            // GIF signature
            return "image/gif".to_string();
        } else if data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && data.get(8..12) == Some(&[0x57, 0x45, 0x42, 0x50]) {
            // WEBP signature
            return "image/webp".to_string();
        } else if data.starts_with(&[0x42, 0x4D]) {
            // BMP signature
            return "image/bmp".to_string();
        } else if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
            // TIFF signature
            return "image/tiff".to_string();
        } else if data.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
            // ICO signature
            return "image/x-icon".to_string();
        }
        
        // Default to jpeg which is most common on Instagram
        "image/jpeg".to_string()
    }
}
