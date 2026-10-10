//! Local background removal and user-selected API generation, both non-destructive.
use photocraft_color::PixelFormat;
use photocraft_doc::{Layer, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};

fn err(message: String) -> EngineError {
    EngineError::Other(message)
}
fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
fn document(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "Open a document first".into())
}
fn selected(s: &Session) -> std::result::Result<(), String> {
    document(s)?;
    if s.active().is_some_and(|st| st.doc.selection.is_some()) { Ok(()) } else { Err("Select the area to generate first".into()) }
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    require_download(p)?;
    crate::jobs::run(
        s,
        "Install Background Model",
        false,
        |ctx| {
            ctx.progress(0.1, "Downloading background model (5 MB)");
            let path = photocraft_ml::install_model(|| ctx.cancelled()).map_err(err)?;
            ctx.check()?;
            Ok(path)
        },
        |_, path| Ok(json!({"installed":true,"path":path})),
    )
}

fn require_download(p: &Value) -> Result<()> {
    if p.get("download").and_then(Value::as_bool) != Some(true) {
        return Err(err("Explicit download request required".into()));
    }
    Ok(())
}

fn cutout(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if layer.surface().is_none() || doc.effective_locks(id).pixels || doc.effective_locks(id).all {
        return Err(err("Select an unlocked pixel layer".into()));
    }
    let replace = p.get("replaceMask").and_then(Value::as_bool).unwrap_or(false);
    if layer.mask.is_some() && !replace {
        return Err(err("This layer already has a mask; duplicate it or enable Replace Existing Mask".into()));
    }
    crate::jobs::edit_job(
        s,
        "AI Remove Background",
        move |doc, _, ctx| {
            let area = doc.bounds();
            let (w, h) = (area.width() as usize, area.height() as usize);
            photocraft_ml::validate_image(w, h, w.saturating_mul(h).saturating_mul(3)).map_err(err)?;
            ctx.progress(0.1, "Preparing image");
            let step = w.max(h).div_ceil(640).max(1);
            let sample = crate::smartselect_cmds::with_doc_sampler(doc, Some(id), false, |sampler, _| sampler.rgb_scaled(area, step));
            let rgb: Vec<u8> = sample.px.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
            ctx.check()?;
            ctx.progress(0.25, "Finding subject locally");
            let mask = photocraft_ml::remove_background(sample.w, sample.h, &rgb, || ctx.cancelled()).map_err(err)?;
            let image = image::GrayImage::from_raw(sample.w as u32, sample.h as u32, mask).ok_or_else(|| err("Invalid mask".into()))?;
            let mask = image::imageops::resize(&image, w as u32, h as u32, image::imageops::FilterType::Triangle);
            ctx.check()?;
            ctx.progress(0.9, "Creating layer mask");
            let mut surface = Surface::new(PixelFormat::GRAY8);
            surface.write_interleaved(area, mask.as_raw());
            crate::extra_cmds::background_to_layer_for_mask(doc, id);
            doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(LayerMask { surface, ..LayerMask::reveal_all() });
            Ok(id)
        },
        |id| json!({"layer":id.0,"model":"u2netp","local":true}),
    )
}

fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    let prompt = p.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_owned();
    if prompt.is_empty() || prompt.len() > 4096 {
        return Err(err("Enter a description (up to 4096 bytes)".into()));
    }
    selected(s).map_err(err)?;
    // Until an ICC conversion bridge is added, never silently reinterpret CMYK/Lab as RGB.
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    if doc.mode != photocraft_color::ColorMode::Rgb {
        return Err(err("Convert the document to RGB before generation".into()));
    }
    crate::jobs::edit_job(
        s,
        "Generative Fill",
        move |doc, active, ctx| {
            let selection = doc.selection.as_ref().ok_or_else(|| err("No selection".into()))?;
            let selected_area = selection.content_bounds().intersect(&doc.bounds());
            if selected_area.is_empty() {
                return Err(err("Selection is empty".into()));
            }
            // Include context, and cap the source crop to bound rendering memory.
            let area = Rect::new(
                selected_area.x0.saturating_sub(64),
                selected_area.y0.saturating_sub(64),
                selected_area.x1.saturating_add(64),
                selected_area.y1.saturating_add(64),
            )
            .intersect(&doc.bounds());
            let (w, h) = (area.width() as usize, area.height() as usize);
            if w.saturating_mul(h) > 16_000_000 {
                return Err(err("Select a smaller area (up to 16 megapixels including context)".into()));
            }
            ctx.progress(0.1, "Preparing selected area");
            let ratio = (1024.0 / w.max(h) as f64).min(1.0);
            let rw = (((w as f64 * ratio).ceil() as u32).div_ceil(8) * 8).clamp(8, 1024);
            let rh = (((h as f64 * ratio).ceil() as u32).div_ceil(8) * 8).clamp(8, 1024);
            let composite = photocraft_compose::render_reduced_rect(doc, area, rw, rh);
            let profile = crate::color_cmds::composite_profile(doc);
            let srgb = photocraft_cms::builtin::srgb();
            let to_srgb = photocraft_cms::Transform::new(&profile, srgb, photocraft_cms::Intent::RelativeColorimetric, true).map_err(|e| err(e.to_string()))?;
            let from_srgb =
                photocraft_cms::Transform::new(srgb, &profile, photocraft_cms::Intent::RelativeColorimetric, true).map_err(|e| err(e.to_string()))?;
            let source: Vec<f32> = composite.px.iter().flat_map(|p| p.iter().take(3).copied()).collect();
            let mut converted = vec![0.0; source.len()];
            to_srgb.convert_f32(&source, 3, &mut converted, 3, false);
            let rgb: Vec<u8> = converted.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            let mask = selection.to_interleaved(area);
            let mask = image::GrayImage::from_raw(w as u32, h as u32, mask).ok_or_else(|| err("Invalid selection mask".into()))?;
            let input_mask = image::imageops::resize(&mask, rw, rh, image::imageops::FilterType::Triangle);
            ctx.check()?;
            ctx.progress(0.25, "Generating with your selected API");
            let result = photocraft_ml::api::generate(&prompt, rw as usize, rh as usize, &rgb, input_mask.as_raw(), || ctx.cancelled()).map_err(err)?;
            ctx.check()?;
            let mut converted = vec![0; result.len()];
            from_srgb.convert_u8(&result, 3, &mut converted, 3, false);
            let output = image::RgbImage::from_raw(rw, rh, converted).ok_or_else(|| err("Invalid generated image".into()))?;
            let output = image::imageops::resize(&output, w as u32, h as u32, image::imageops::FilterType::Triangle);
            let mut layer = Layer::raster("Генеративная заливка", doc.pixel_format());
            let surface = layer.surface_mut().ok_or_else(|| err("Cannot create result layer".into()))?;
            // Keep the original selection as alpha: pixels outside it are never changed.
            for (row, pixels) in output.as_raw().chunks_exact(w * 3).enumerate() {
                ctx.check()?;
                for (x, pixel) in pixels.chunks_exact(3).enumerate() {
                    let dx = area.x0 + x as i32;
                    let dy = area.y0 + row as i32;
                    let alpha = selection.pixel(dx, dy).first().copied().unwrap_or(0.0);
                    let mut rgba: Vec<f32> = pixel.iter().map(|v| f32::from(*v) / 255.0).collect();
                    rgba.push(alpha);
                    surface.write_pixel(dx, dy, &rgba);
                }
            }
            let id = doc.insert_above(*active, layer);
            *active = Some(id);
            Ok(id)
        },
        |id| json!({"layer":id.0,"provider":photocraft_ml::api::config().model}),
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "ai.installBackgroundModel",
            label: "Install Background Model",
            menu: &["Filter", "AI Tools"],
            shortcut: None,
            params: r#"{"download":bool=false}"#,
            enabled: always,
            run: install,
            journal: false,
        },
        CommandSpec {
            id: "ai.removeBackground",
            label: "AI Remove Background",
            menu: &["Filter", "AI Tools"],
            shortcut: None,
            params: r#"{"replaceMask":bool=false}"#,
            enabled: document,
            run: cutout,
            journal: true,
        },
        CommandSpec {
            id: "ai.generativeFill",
            label: "Generative Fill…",
            menu: &["Edit"],
            shortcut: None,
            params: r#"{"prompt":text}"#,
            enabled: selected,
            run: generate,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ai_commands_reject_missing_context_and_bad_parameters() {
        let mut s = Session::new();
        assert!(s.execute("ai.installBackgroundModel", json!({})).is_err());
        assert!(!s.is_enabled("ai.removeBackground"));
        assert!(!s.is_enabled("ai.generativeFill"));
        assert!(s.execute("ai.generativeFill", json!({"prompt":""})).is_err());
        s.execute("file.new", json!({"width":16,"height":16})).unwrap();
        s.edit("existing mask", |doc, _| {
            doc.layers[0].mask = Some(LayerMask::reveal_all());
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute("ai.removeBackground", json!({})).is_err());
        assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
        assert_eq!(s.active().unwrap().doc.layers[0].mask, before.layers[0].mask);
    }
}
