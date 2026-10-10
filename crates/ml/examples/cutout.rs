//! Run a real model end to end, with timings and a transparent PNG for visual inspection.
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let input = args.get(1).ok_or("Usage: cutout input.png output.png")?;
    let output = args.get(2).ok_or("Usage: cutout input.png output.png")?;
    let image = image::open(input).map_err(|e| e.to_string())?.to_rgb8();
    let start = std::time::Instant::now();
    photocraft_ml::install_model(|| false)?;
    let mask = photocraft_ml::remove_background(image.width() as usize, image.height() as usize, image.as_raw(), || false)?;
    let mut rgba = Vec::with_capacity(mask.len() * 4);
    for (pixel, alpha) in image.pixels().zip(mask) {
        rgba.extend_from_slice(&pixel.0);
        rgba.push(alpha);
    }
    image::save_buffer(output, &rgba, image.width(), image.height(), image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    let preview: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let a = f32::from(pixel[3]) / 255.0;
            pixel[..3].iter().map(move |v| (f32::from(*v) * a + 48.0 * (1.0 - a)).round() as u8)
        })
        .collect();
    image::save_buffer(format!("{output}.preview.png"), &preview, image.width(), image.height(), image::ColorType::Rgb8).map_err(|e| e.to_string())?;
    println!("Cutout completed in {:.2}s", start.elapsed().as_secs_f64());
    Ok(())
}
