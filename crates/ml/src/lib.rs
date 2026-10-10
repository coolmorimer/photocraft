//! On-demand local intelligence. No process, model or network request at startup.
//! U²-NetP runs on CPU in pure Rust. Only generation uses a user-selected remote API.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(not(target_arch = "wasm32"))]
pub mod api;
#[cfg(not(target_arch = "wasm32"))]
pub use api::{ApiConfig, Protocol};

pub type Result<T> = std::result::Result<T, String>;

/// Bound allocation and work even when called through MCP with hostile dimensions.
pub fn validate_image(width: usize, height: usize, len: usize) -> Result<()> {
    let pixels = width.checked_mul(height).ok_or("Image dimensions overflow")?;
    if width == 0 || height == 0 || pixels > 40_000_000 || pixels.checked_mul(3) != Some(len) {
        return Err("Expected an RGB image of 1–40 million pixels".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_or_unbounded_images() {
        assert!(validate_image(usize::MAX, 2, 0).is_err());
        assert!(validate_image(0, 10, 0).is_err());
        assert!(validate_image(10, 10, 299).is_err());
        assert!(validate_image(100_000, 100_000, 0).is_err());
        assert!(validate_image(320, 320, 320 * 320 * 3).is_ok());
    }
}
