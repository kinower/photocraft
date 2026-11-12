//! GenFill-K: the Ai Fill gallery with Codex ImageGen (`/imagen`) instead of the local engine.
//! The context crop and a copy with the selection outlined in red go to Codex; the result is
//! scaled back to the crop and shown through the layer mask (selection grown and feathered).
use crate::{EngineError, Result, jobs::JobCtx};
use photocraft_codecs::{ChannelLayout, Image, SampleType};
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

pub const ENGINE: &str = "imagen";
pub const LABEL: &str = "GenFill-K";
/// Context around the selection sent to Codex.
pub const MARGIN: i32 = 96;
/// The pasted area: the selection grown by `GROW` px and feathered by `FEATHER` px.
pub const GROW: usize = 10;
pub const FEATHER: f32 = 4.0;
const OUTLINE: u32 = 3;

fn error(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("GenFill-K: {message}"))
}

/// The selection grown by 10 px and feathered by 4 px, as a layer-mask surface.
pub(crate) fn grown_mask(selection: &Surface, canvas: Rect) -> Result<Surface> {
    if selection.default_pixel().first().copied().unwrap_or(0.0) > 0.0 {
        return Ok(selection.clone());
    }
    let bounds = selection.content_bounds().intersect(&canvas);
    if bounds.is_empty() {
        return Err(error("the mask is empty"));
    }
    let rect = bounds.inflate(GROW as i32 + (FEATHER * 3.0).ceil() as i32 + 1).intersect(&canvas);
    if u64::from(rect.width()) * u64::from(rect.height()) > 64 << 20 {
        return Err(error("the selection is too large"));
    }
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let channels = selection.channels().max(1);
    let coverage: Vec<f32> = selection.read_region(rect).chunks_exact(channels).map(|p| p.first().copied().unwrap_or(0.0).clamp(0.0, 1.0)).collect();
    let hard: Vec<bool> = coverage.iter().map(|v| *v >= 0.5).collect();
    let grown = photocraft_algo::remove::dilate(w, h, &hard, GROW);
    let merged: Vec<f32> = grown.iter().zip(&coverage).map(|(g, c)| if *g { 1.0 } else { *c }).collect();
    let soft = photocraft_algo::selection::feather(&merged, w, h, FEATHER);
    let mut surface = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
    surface.write_region(rect, &soft);
    Ok(surface)
}

/// The crop with the selection's contour drawn in red, Codex's second reference.
pub(crate) fn outlined(image: &Image, hole: &Image) -> Result<Image> {
    let image = image.convert(ChannelLayout::Rgba, SampleType::U8);
    let hole = hole.convert(ChannelLayout::Gray, SampleType::U8);
    if image.dimensions() != hole.dimensions() {
        return Err(error("the mask does not match the image"));
    }
    let (w, h) = image.dimensions();
    let inside = |x: i64, y: i64| x >= 0 && y >= 0 && x < i64::from(w) && y < i64::from(h) && hole.data()[(y as usize) * w as usize + x as usize] >= 128;
    let reach = i64::from(OUTLINE);
    let mut data = image.data().to_vec();
    for y in 0..i64::from(h) {
        for x in 0..i64::from(w) {
            let me = inside(x, y);
            let edge = (-reach..=reach).any(|dy| (-reach..=reach).any(|dx| (dx != 0 || dy != 0) && inside(x + dx, y + dy) != me));
            if edge && !me {
                let at = (y as usize * w as usize + x as usize) * 4;
                data[at..at + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
    }
    Image::from_u8(w, h, ChannelLayout::Rgba, data).map_err(error)
}

/// Opaque copy: Codex gets the composite as it looks, without transparency.
fn opaque(image: &Image) -> Result<Image> {
    let image = image.convert(ChannelLayout::Rgba, SampleType::U8);
    let mut data = image.data().to_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    Image::from_u8(image.width(), image.height(), ChannelLayout::Rgba, data).map_err(error)
}

/// Bilinear resize of any Codex result to the crop.
pub(crate) fn fit(image: &Image, size: (u32, u32)) -> Result<Image> {
    let image = image.convert(ChannelLayout::Rgba, SampleType::U8);
    if image.dimensions() == size {
        return Ok(image);
    }
    let (sw, sh) = (image.width() as usize, image.height() as usize);
    let (w, h) = (size.0 as usize, size.1 as usize);
    let src: Vec<f32> = image.data().iter().map(|v| f32::from(*v)).collect();
    let out = photocraft_algo::seam::resize_bilinear(sw, sh, 4, &src, w, h);
    let data = out.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect();
    Image::from_u8(size.0, size.1, ChannelLayout::Rgba, data).map_err(error)
}

pub(crate) fn instruction(prompt: &str, size: (u32, u32)) -> String {
    let what = if prompt.trim().is_empty() {
        "Remove whatever is there and continue the surrounding background seamlessly.".to_owned()
    } else {
        format!("Inside that area: {}.", prompt.trim().trim_end_matches('.'))
    };
    format!(
        "Edit the first image. The second image is the same picture with one area outlined in red. {what} \
         Change only the pixels inside the red outline and keep everything else identical: same framing, \
         perspective, camera position, lighting, colours and grain. Match the scale, light direction and \
         shadows of the scene. Do not draw the red outline, text or borders. Keep the {}x{} aspect ratio \
         of the first image.",
        size.0, size.1
    )
}

fn script() -> Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("GENFILL_K_SCRIPT") {
        return Ok(path.into());
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).ok_or_else(|| error("the user folder is unknown"))?;
    let path = std::path::Path::new(&home).join(".codex/skills/imagen/scripts/generate-with-codex-imagegen.mjs");
    if path.is_file() { Ok(path) } else { Err(error("the /imagen skill is not installed for Codex")) }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn generate(_: &Image, _: &Image, _: &JobCtx, _: &str) -> Result<(Image, Image)> {
    Err(error("Codex ImageGen requires the desktop app"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn generate(image: &Image, hole: &Image, ctx: &JobCtx, prompt: &str) -> Result<(Image, Image)> {
    use photocraft_codecs::{DecodeOptions, EncodeOptions, Format, Limits};
    use std::{
        fs,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    if std::env::var_os("GENSTUDIO_NO_PROCESS").is_some_and(|v| v == "1") {
        return Err(error("process launch is disabled for validation"));
    }
    ctx.check()?;
    let script = script()?;
    let folder = tempfile::Builder::new().prefix("photocraft-genfill-k-").tempdir().map_err(error)?;
    let (input, reference, result) = (folder.path().join("input.png"), folder.path().join("outline.png"), folder.path().join("result.png"));
    let base = opaque(image)?;
    for (path, data) in [(&input, base.clone()), (&reference, outlined(&base, hole)?)] {
        fs::write(path, photocraft_codecs::encode(&data, Format::Png, &EncodeOptions::default()).map_err(error)?).map_err(error)?;
    }
    ctx.progress(0.15, "Generating with Codex ImageGen…");
    let log = fs::File::create(folder.path().join("imagen.log")).map_err(error)?;
    let mut command = Command::new("node");
    command
        .arg(&script)
        .arg("--prompt")
        .arg(instruction(prompt, image.dimensions()))
        .arg("--image")
        .arg(&input)
        .arg("--image")
        .arg(&reference)
        .args(["--wb", "off", "--timeout-ms", "900000"])
        .arg("--out")
        .arg(&result)
        .current_dir(folder.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(error)?))
        .stderr(Stdio::from(log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| error(format!("Node.js could not start the /imagen helper: {e}")))?;
    let started = Instant::now();
    let status = loop {
        if ctx.check().is_err() || started.elapsed() > Duration::from_secs(960) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if started.elapsed() > Duration::from_secs(960) {
                error("Codex took too long; the document is unchanged")
            } else {
                EngineError::Cancelled
            });
        }
        if let Some(status) = child.try_wait().map_err(error)? {
            break status;
        }
        let seconds = started.elapsed().as_secs();
        ctx.progress(0.15 + 0.7 * (seconds as f32 / 180.0).min(1.0), &format!("Generating with Codex ImageGen… {seconds} s"));
        std::thread::sleep(Duration::from_millis(250));
    };
    if !status.success() || !result.is_file() {
        let detail = fs::read_to_string(folder.path().join("imagen.log")).unwrap_or_default();
        let tail: String = detail.lines().rev().filter(|l| !l.trim().is_empty()).take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" · ");
        return Err(error(format!("Codex ImageGen failed: {}", tail.chars().take(600).collect::<String>())));
    }
    ctx.check()?;
    ctx.progress(0.9, "Fitting the result to the selection…");
    let bytes = fs::read(&result).map_err(error)?;
    let limits = Limits { max_width: 8192, max_height: 8192, max_pixels: 64 << 20, max_alloc: 512 << 20 };
    let generated = photocraft_codecs::decode_as_with(Format::Png, &bytes, &DecodeOptions { limits, ..Default::default() }).map_err(error)?;
    Ok((fit(&generated, image.dimensions())?, hole.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grown_mask_covers_ten_pixels_and_fades_out_by_the_feather() {
        let mut selection = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        selection.write_region(Rect::new(100, 100, 140, 130), &vec![1.0; 40 * 30]);
        let mask = grown_mask(&selection, Rect::new(0, 0, 400, 300)).unwrap();
        let at = |x, y| mask.read_region(Rect::new(x, y, x + 1, y + 1))[0];
        assert!(at(120, 115) > 0.99, "inside stays fully revealed");
        assert!(at(97, 115) > 0.95, "3 px outside is pasted");
        assert!(at(89, 115) > 0.3 && at(89, 115) < 0.7, "the edge 10 px out is soft");
        assert!(at(70, 115) < 0.01, "far outside is untouched");
        assert!(grown_mask(&Surface::with_default(PixelFormat::GRAY8, &[0.0]), Rect::new(0, 0, 10, 10)).is_err());
    }

    #[test]
    fn outline_is_red_just_outside_the_selection_and_leaves_the_rest() {
        let image = Image::from_u8(20, 20, ChannelLayout::Rgba, vec![10; 20 * 20 * 4]).unwrap();
        let hole = Image::from_u8(
            20,
            20,
            ChannelLayout::Gray,
            (0..400).map(|i| if (5..15).contains(&(i % 20)) && (5..15).contains(&(i / 20)) { 255 } else { 0 }).collect(),
        )
        .unwrap();
        let out = outlined(&image, &hole).unwrap();
        let px = |x: usize, y: usize| &out.data()[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
        assert_eq!(px(4, 10), &[255, 0, 0, 255]);
        assert_eq!(px(10, 10), &[10, 10, 10, 10], "the selection itself is not painted");
        assert_eq!(px(0, 0), &[10, 10, 10, 10]);
    }

    #[test]
    fn results_of_any_size_are_fitted_to_the_crop() {
        let wide = Image::from_u8(1536, 1024, ChannelLayout::Rgba, vec![200; 1536 * 1024 * 4]).unwrap();
        let fitted = fit(&wide, (288, 224)).unwrap();
        assert_eq!(fitted.dimensions(), (288, 224));
        assert!(fitted.data().iter().all(|v| *v == 200));
        assert!(instruction("", (288, 224)).contains("288x224"));
        assert!(instruction("an olive tree in a pot.", (1, 1)).contains("Inside that area: an olive tree in a pot."));
    }
}
