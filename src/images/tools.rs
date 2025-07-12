use thiserror::Error;
use image::{DynamicImage, GenericImageView};
use image::imageops::FilterType;
use serde::{Deserialize, Serialize};

#[derive(Error, Debug)]
pub enum ImageProxyError {
    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),
    
    #[error("Proxy error: {0}")]
    ProxyError(String),
    
    #[error("Image error: {0}")]
    ImageError(String),
    
    #[error("Image conversion error: {0}")]
    ConversionError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImageConversionFormat {
    Webp,
    Jpg,
    Png,
    Gif,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ImageFit {
    Pad,
    Fill,
    Scale,
    Crop,
    Thumb,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageFocus {
    Center,
    Top,
    Right,
    Left,
    Bottom,
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
    Face,
    Faces,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageConversionParams {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<ImageConversionFormat>,
    pub quality: Option<u8>,
    pub fit: Option<ImageFit>,
    pub focus: Option<ImageFocus>,
}

impl ImageConversionParams {
    pub fn to_cache_key(&self) -> String {
        let mut parts = Vec::new();
        
        if let Some(width) = self.width {
            parts.push(format!("w{}", width));
        }
        if let Some(height) = self.height {
            parts.push(format!("h{}", height));
        }
        if let Some(ref format) = self.format {
            parts.push(format!("f{:?}", format).to_lowercase());
        }
        if let Some(quality) = self.quality {
            parts.push(format!("q{}", quality));
        }
        if let Some(ref fit) = self.fit {
            parts.push(format!("fit{:?}", fit).to_lowercase());
        }
        if let Some(ref focus) = self.focus {
            parts.push(format!("focus{:?}", focus).to_lowercase().replace("_", ""));
        }
        
        if parts.is_empty() {
            "original".to_string()
        } else {
            parts.join("_")
        }
    }
    
    /// Check if any conversion parameters are set (i.e., if conversion is needed)
    pub fn needs_conversion(&self) -> bool {
        self.width.is_some() || self.height.is_some() || self.format.is_some() 
        || self.quality.is_some() || self.fit.is_some() || self.focus.is_some()
    }
}

// Convert image according to parameters
pub fn convert_image(
    image_data: Vec<u8>,
    params: &ImageConversionParams,
) -> Result<(Vec<u8>, String), ImageProxyError> {
    // Load the image
    let img = image::load_from_memory(&image_data)
        .map_err(|e| ImageProxyError::ConversionError(format!("Failed to load image: {}", e)))?;
    
    // Apply transformations
    let processed_img = apply_transformations(img, params)?;
    
    // Convert to desired format
    let (output_data, content_type) = encode_image(processed_img, params)?;
    
    Ok((output_data, content_type))
}

fn apply_transformations(
    mut img: DynamicImage,
    params: &ImageConversionParams,
) -> Result<DynamicImage, ImageProxyError> {
    // Apply resizing if width or height is specified
    if params.width.is_some() || params.height.is_some() {
        img = resize_image(img, params)?;
    }
    
    Ok(img)
}

fn resize_image(
    img: DynamicImage,
    params: &ImageConversionParams,
) -> Result<DynamicImage, ImageProxyError> {
    let (current_width, current_height) = img.dimensions();
    
    // Calculate target dimensions
    let (target_width, target_height) = match (params.width, params.height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => {
            let aspect_ratio = current_height as f64 / current_width as f64;
            (w, (w as f64 * aspect_ratio) as u32)
        },
        (None, Some(h)) => {
            let aspect_ratio = current_width as f64 / current_height as f64;
            ((h as f64 * aspect_ratio) as u32, h)
        },
        (None, None) => return Ok(img), // No resizing needed
    };
    
    // Apply fit strategy
    let fit_strategy = params.fit.as_ref().unwrap_or(&ImageFit::Scale);
    
    let resized_img = match fit_strategy {
        ImageFit::Scale => {
            // Scale to exact dimensions (may distort aspect ratio)
            img.resize_exact(target_width, target_height, FilterType::Lanczos3)
        },
        ImageFit::Fill => {
            // Scale to fill the target dimensions, then crop with focus
            let (current_width, current_height) = img.dimensions();
            
            // Calculate scaling factor to fill the target dimensions
            let scale_x = target_width as f64 / current_width as f64;
            let scale_y = target_height as f64 / current_height as f64;
            let scale = scale_x.max(scale_y); // Use the larger scale to fill
            
            // Scale the image
            let scaled_width = (current_width as f64 * scale) as u32;
            let scaled_height = (current_height as f64 * scale) as u32;
            let scaled_img = img.resize(scaled_width, scaled_height, FilterType::Lanczos3);
            
            // Now crop from the scaled image using the focus point
            crop_image(scaled_img, target_width, target_height, params.focus.as_ref())?
        },
        ImageFit::Crop => {
            // Crop to exact dimensions from center or focus point
            crop_image(img, target_width, target_height, params.focus.as_ref())?
        },
        ImageFit::Pad => {
            // Resize to fit within dimensions, padding if necessary
            let resized = img.resize(target_width, target_height, FilterType::Lanczos3);
            pad_image(resized, target_width, target_height)?
        },
        ImageFit::Thumb => {
            // Create thumbnail (resize to fit) with high quality filter
            img.resize(target_width, target_height, FilterType::Lanczos3)
        },
    };
    
    Ok(resized_img)
}

fn crop_image(
    img: DynamicImage,
    target_width: u32,
    target_height: u32,
    focus: Option<&ImageFocus>,
) -> Result<DynamicImage, ImageProxyError> {
    let (current_width, current_height) = img.dimensions();
    
    // Calculate crop position based on focus
    let (crop_x, crop_y) = match focus.unwrap_or(&ImageFocus::Center) {
        ImageFocus::Center => (
            (current_width.saturating_sub(target_width)) / 2,
            (current_height.saturating_sub(target_height)) / 2,
        ),
        ImageFocus::Top => (
            (current_width.saturating_sub(target_width)) / 2,
            0,
        ),
        ImageFocus::Bottom => (
            (current_width.saturating_sub(target_width)) / 2,
            current_height.saturating_sub(target_height),
        ),
        ImageFocus::Left => (
            0,
            (current_height.saturating_sub(target_height)) / 2,
        ),
        ImageFocus::Right => (
            current_width.saturating_sub(target_width),
            (current_height.saturating_sub(target_height)) / 2,
        ),
        ImageFocus::TopLeft => (0, 0),
        ImageFocus::TopRight => (current_width.saturating_sub(target_width), 0),
        ImageFocus::BottomLeft => (0, current_height.saturating_sub(target_height)),
        ImageFocus::BottomRight => (
            current_width.saturating_sub(target_width),
            current_height.saturating_sub(target_height),
        ),
        ImageFocus::Face | ImageFocus::Faces => {
            // For face detection, fall back to center for now
            // This could be enhanced with face detection libraries
            (
                (current_width.saturating_sub(target_width)) / 2,
                (current_height.saturating_sub(target_height)) / 2,
            )
        },
    };
    
    // Ensure crop dimensions don't exceed image bounds
    let crop_width = target_width.min(current_width);
    let crop_height = target_height.min(current_height);
    
    Ok(img.crop_imm(crop_x, crop_y, crop_width, crop_height))
}

fn pad_image(
    img: DynamicImage,
    target_width: u32,
    target_height: u32,
) -> Result<DynamicImage, ImageProxyError> {
    let (current_width, current_height) = img.dimensions();
    
    if current_width == target_width && current_height == target_height {
        return Ok(img);
    }
    
    // Create a new image with the target dimensions and transparent background
    let mut padded = DynamicImage::new_rgba8(target_width, target_height);
    
    // Calculate position to center the image
    let x_offset = (target_width.saturating_sub(current_width)) / 2;
    let y_offset = (target_height.saturating_sub(current_height)) / 2;
    
    // Overlay the original image onto the padded canvas
    image::imageops::overlay(&mut padded, &img, x_offset as i64, y_offset as i64);
    
    Ok(padded)
}

fn encode_image(
    img: DynamicImage,
    params: &ImageConversionParams,
) -> Result<(Vec<u8>, String), ImageProxyError> {
    let mut output = Vec::new();
    let format = params.format.as_ref().unwrap_or(&ImageConversionFormat::Jpg);
    
    match format {
        ImageConversionFormat::Webp => {
            // WebP encoding using the image crate's standard API
            img.write_to(&mut std::io::Cursor::new(&mut output), image::ImageFormat::WebP)
                .map_err(|e| ImageProxyError::ConversionError(format!("WebP encoding failed: {}", e)))?;
            
            Ok((output, "image/webp".to_string()))
        },
        ImageConversionFormat::Jpg => {
            let quality = params.quality.unwrap_or(85).min(100);
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality);
            encoder.encode_image(&img)
                .map_err(|e| ImageProxyError::ConversionError(format!("JPEG encoding failed: {}", e)))?;
            
            Ok((output, "image/jpeg".to_string()))
        },
        ImageConversionFormat::Png => {
            img.write_to(&mut std::io::Cursor::new(&mut output), image::ImageFormat::Png)
                .map_err(|e| ImageProxyError::ConversionError(format!("PNG encoding failed: {}", e)))?;
            
            Ok((output, "image/png".to_string()))
        },
        ImageConversionFormat::Gif => {
            img.write_to(&mut std::io::Cursor::new(&mut output), image::ImageFormat::Gif)
                .map_err(|e| ImageProxyError::ConversionError(format!("GIF encoding failed: {}", e)))?;
            
            Ok((output, "image/gif".to_string()))
        },
    }
} 