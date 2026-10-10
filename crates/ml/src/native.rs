use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use image::{RgbImage, imageops};
use sha2::{Digest, Sha256};
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::infer::InferenceOp;

use crate::Result;

pub const MODEL_URL: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx";
pub const MODEL_SHA256: &str = "309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8";
const MODEL_BYTES: u64 = 6 * 1024 * 1024;
pub(crate) static WORKER: Mutex<()> = Mutex::new(());

fn resize_with_optional_scales(
    ctx: &tract_onnx::model::ParsingContext<'_>,
    node: &tract_onnx::pb::NodeProto,
) -> TractResult<(Box<dyn InferenceOp>, Vec<String>)> {
    // Resize-11 and Resize-13 have the same inputs here. Use tract's newer parser
    // so an absent optional scales input stays absent during evaluation.
    let framework = tract_onnx::onnx();
    let builder = framework.op_register.0.get("Resize").ok_or_else(|| anyhow::anyhow!("Resize operator unavailable"))?;
    let mut context = ctx.clone();
    context.onnx_operator_set_version = 13;
    builder(&context, node)
}

pub fn model_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_MODELS_DIR") {
        return Ok(PathBuf::from(dir).join("u2netp.onnx"));
    }
    let dirs = directories::ProjectDirs::from("io", "coolmorimer", "PhotoCraft-RU").ok_or("Cannot locate model cache")?;
    Ok(dirs.cache_dir().join("models/u2netp.onnx"))
}

fn verified(path: &std::path::Path) -> Result<bool> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if file.metadata().map_err(|e| e.to_string())?.len() > MODEL_BYTES {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(buffer.get(..n).ok_or("Invalid read length")?);
    }
    Ok(format!("{:x}", hash.finalize()) == MODEL_SHA256)
}

/// Download only when the user requests it. Bounded, verified, and atomically installed.
pub fn install_model(cancelled: impl Fn() -> bool) -> Result<PathBuf> {
    let _guard = WORKER.try_lock().map_err(|_| "An AI operation is already running")?;
    let path = model_path()?;
    if verified(&path)? {
        return Ok(path);
    }
    let parent = path.parent().ok_or("Invalid model cache path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let client =
        reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(180)).build().map_err(|e| e.to_string())?;
    let mut response = client.get(MODEL_URL).send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        if cancelled() {
            return Err("Cancelled".into());
        }
        let n = response.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if data.len().saturating_add(n) as u64 > MODEL_BYTES {
            return Err("Model exceeds download limit".into());
        }
        data.extend_from_slice(buffer.get(..n).ok_or("Invalid download length")?);
    }
    if format!("{:x}", Sha256::digest(&data)) != MODEL_SHA256 {
        return Err("Model checksum mismatch".into());
    }
    let temp = path.with_extension("onnx.part");
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&data).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
    drop(file);
    // An invalid old cache is replaceable; never overwrite a valid model in use.
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// U²-NetP preprocessing: 320² RGB, max normalization followed by ImageNet mean/std.
/// The runnable is deliberately dropped after the operation, releasing model memory.
pub fn remove_background(width: usize, height: usize, rgb: &[u8], cancelled: impl Fn() -> bool) -> Result<Vec<u8>> {
    crate::validate_image(width, height, rgb.len())?;
    let _guard = WORKER.try_lock().map_err(|_| "An AI operation is already running")?;
    let path = model_path()?;
    if !verified(&path)? {
        return Err("Install the background model from AI Tools first".into());
    }
    if cancelled() {
        return Err("Cancelled".into());
    }
    let image = RgbImage::from_raw(width as u32, height as u32, rgb.to_vec()).ok_or("Invalid image buffer")?;
    let resized = imageops::resize(&image, 320, 320, imageops::FilterType::Triangle);
    let max = f32::from(resized.as_raw().iter().copied().max().unwrap_or(1)).max(1.0);
    let means = [0.485f32, 0.456, 0.406];
    let stds = [0.229f32, 0.224, 0.225];
    let mut values = Vec::with_capacity(3 * 320 * 320);
    for (c, (&mean, &std)) in means.iter().zip(&stds).enumerate() {
        for px in resized.pixels() {
            values.push((f32::from(px.0.get(c).copied().unwrap_or(0)) / max - mean) / std);
        }
    }
    let mut onnx = tract_onnx::onnx();
    onnx.op_register.insert("Resize", resize_with_optional_scales);
    let mut proto = onnx.proto_model_for_path(path).map_err(|e| format!("Cannot read background model: {e:#}"))?;
    // The verified U2NetP export only resizes to dimensions >= 20. ONNX's
    // pytorch_half_pixel and half_pixel are identical whenever output length > 1.
    // tract 0.22 accepts the latter spelling; no weights or on-disk model change.
    if let Some(graph) = &mut proto.graph {
        for node in &mut graph.node {
            if node.op_type == "Resize" {
                // This export supplies an empty scales tensor together with sizes.
                // ONNX treats it as absent; tract's evaluator otherwise iterates
                // over that empty tensor and leaves the input unresized.
                if node.input.get(3).is_some_and(|s| !s.is_empty())
                    && let Some(scales) = node.input.get_mut(2)
                {
                    scales.clear();
                }
                for attr in &mut node.attribute {
                    if attr.name == "coordinate_transformation_mode" && attr.s == b"pytorch_half_pixel" {
                        attr.s = b"half_pixel".to_vec();
                    }
                }
            }
        }
    }
    let model = onnx
        .model_for_proto_model(&proto)
        .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, 320, 320]).into()))
        .and_then(|m| m.into_optimized())
        .and_then(|m| m.into_runnable())
        .map_err(|e| format!("Cannot load background model: {e:#}"))?;
    if cancelled() {
        return Err("Cancelled".into());
    }
    let input = Tensor::from_shape(&[1, 3, 320, 320], &values).map_err(|e| e.to_string())?;
    let outputs = model.run(tvec!(input.into())).map_err(|e| format!("Background inference failed: {e}"))?;
    if cancelled() {
        return Err("Cancelled".into());
    }
    let output = outputs.first().ok_or("Model returned no mask")?.to_array_view::<f32>().map_err(|e| e.to_string())?;
    if output.len() != 320 * 320 || output.iter().any(|v| !v.is_finite()) {
        return Err("Invalid model mask".into());
    }
    let min = output.iter().copied().fold(f32::INFINITY, f32::min);
    let max = output.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = max - min;
    if span < 1e-6 {
        return Err("No distinct subject found".into());
    }
    let bytes: Vec<u8> = output.iter().map(|v| (((v - min) / span).clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    let mask = image::GrayImage::from_raw(320, 320, bytes).ok_or("Invalid mask buffer")?;
    Ok(imageops::resize(&mask, width as u32, height as u32, imageops::FilterType::Triangle).into_raw())
}
