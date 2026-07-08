use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use image::{GenericImageView, ImageReader, imageops::FilterType};

const PREVIEW_BG: [u8; 3] = [11, 15, 20];

#[derive(Debug, Clone)]
pub struct ImagePreview {
    pub title: String,
    pub path: PathBuf,
    pub source_width: u32,
    pub source_height: u32,
    pub cell_width: u16,
    pub cell_height: u16,
    pub rows: Vec<Vec<ImageCell>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageCell {
    pub upper: [u8; 3],
    pub lower: [u8; 3],
}

pub fn load_image_preview(
    path: impl AsRef<Path>,
    title: Option<String>,
    max_cell_width: u16,
    max_cell_height: u16,
) -> Result<ImagePreview> {
    let path = path.as_ref();
    let image = ImageReader::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .with_guessed_format()
        .context("failed to detect image format")?
        .decode()
        .with_context(|| format!("failed to decode image {}", path.display()))?;
    let (source_width, source_height) = image.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err(anyhow!("image has no pixels"));
    }

    let max_pixel_width = u32::from(max_cell_width.max(1));
    let max_pixel_height = u32::from(max_cell_height.max(1)) * 2;
    let (target_width, target_height) = fit_dimensions(
        source_width,
        source_height,
        max_pixel_width,
        max_pixel_height,
    );
    let resized = image
        .resize_exact(target_width, target_height, FilterType::Triangle)
        .to_rgba8();

    let cell_width = target_width as u16;
    let cell_height = target_height.div_ceil(2) as u16;
    let mut rows = Vec::with_capacity(cell_height as usize);
    for cell_y in 0..cell_height {
        let upper_y = u32::from(cell_y) * 2;
        let lower_y = (upper_y + 1).min(target_height - 1);
        let mut row = Vec::with_capacity(cell_width as usize);
        for x in 0..target_width {
            row.push(ImageCell {
                upper: composite_rgba(resized.get_pixel(x, upper_y).0),
                lower: composite_rgba(resized.get_pixel(x, lower_y).0),
            });
        }
        rows.push(row);
    }

    let title = title
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .or_else(|| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "image".into());

    Ok(ImagePreview {
        title,
        path: path.to_path_buf(),
        source_width,
        source_height,
        cell_width,
        cell_height,
        rows,
    })
}

fn fit_dimensions(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    let width_scale = max_width as f64 / width as f64;
    let height_scale = max_height as f64 / height as f64;
    let scale = width_scale.min(height_scale).clamp(0.01, 1.0);
    let target_width = ((width as f64) * scale).round().max(1.0) as u32;
    let target_height = ((height as f64) * scale).round().max(1.0) as u32;
    (
        target_width.min(max_width.max(1)),
        target_height.min(max_height.max(1)),
    )
}

fn composite_rgba(pixel: [u8; 4]) -> [u8; 3] {
    let alpha = pixel[3] as u16;
    [
        blend_channel(pixel[0], PREVIEW_BG[0], alpha),
        blend_channel(pixel[1], PREVIEW_BG[1], alpha),
        blend_channel(pixel[2], PREVIEW_BG[2], alpha),
    ]
}

fn blend_channel(fg: u8, bg: u8, alpha: u16) -> u8 {
    (((fg as u16 * alpha) + (bg as u16 * (255 - alpha))) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_dimensions_preserves_aspect_ratio_inside_cell_budget() {
        assert_eq!(fit_dimensions(400, 200, 100, 60), (100, 50));
        assert_eq!(fit_dimensions(200, 400, 100, 60), (30, 60));
    }

    #[test]
    fn transparent_pixels_blend_against_preview_background() {
        assert_eq!(composite_rgba([255, 0, 0, 0]), PREVIEW_BG);
        assert_eq!(composite_rgba([255, 0, 0, 255]), [255, 0, 0]);
    }
}
