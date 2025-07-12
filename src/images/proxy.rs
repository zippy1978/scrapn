use std::time::Duration;
use crate::images::tools::{ImageProxyError, ImageConversionParams};

pub struct ImageProxy {
    timeout: Duration,
}

impl ImageProxy {
    pub fn new(timeout: u64) -> Self {
        Self {
            timeout: Duration::from_secs(timeout),
        }
    }
    
    // Fetch and optionally convert an image
    pub async fn fetch_and_convert_image(
        &self,
        url: &str,
        params: &ImageConversionParams,
    ) -> Result<(Vec<u8>, String), ImageProxyError> {
        // First fetch the original image
        let (original_data, original_content_type) = self.fetch_image(url).await?;
        
        // If no conversion params, return original
        if params.width.is_none() && params.height.is_none() && params.format.is_none() 
           && params.quality.is_none() && params.fit.is_none() && params.focus.is_none() {
            return Ok((original_data, original_content_type));
        }
        
        // Convert the image using tools
        crate::images::tools::convert_image(original_data, params)
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
                    log::error!("Image request failed with status: {}", status);
                    return Err(ImageProxyError::ImageError(
                        format!("Image request failed with status: {}", status)
                    ));
                }

                log::info!("Image request successful");
                
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
        // Check for common image file signatures
        if data.len() >= 4 {
            if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
                return "image/jpeg".to_string();
            } else if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
                return "image/png".to_string();
            } else if data.starts_with(&[0x47, 0x49, 0x46]) {
                return "image/gif".to_string();
            } else if data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && data.len() >= 12 {
                // Check for WEBP signature
                if data[8..12] == [0x57, 0x45, 0x42, 0x50] {
                    return "image/webp".to_string();
                }
            }
        }
        
        // Default to JPEG if we can't detect
        "image/jpeg".to_string()
    }
} 