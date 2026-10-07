//! The picture the screenshot tool makes: the selection cut out of the frozen screen, with the
//! annotations drawn into it. The annotations are rendered from an SVG by resvg, in the same
//! colours, widths and fonts the overlay showed (ui/shot.slint), so that the picture sent is the
//! picture seen, whatever theme the other side uses.

use std::io::Cursor;

use super::{Annotation, Display, Rect, Shape, color_hex, halo_hex};

/// The selection `rect` (in points of `display`) with `annotations`, as PNG.
pub fn compose(display: &Display, rect: Rect, annotations: &[Annotation]) -> Option<Vec<u8>> {
    let scale = display.scale;
    let (frame_w, frame_h) = display.px;
    let x0 = ((rect.x * scale).round().max(0.0) as u32).min(frame_w.saturating_sub(1));
    let y0 = ((rect.y * scale).round().max(0.0) as u32).min(frame_h.saturating_sub(1));
    let w = (((rect.x + rect.w) * scale).round() as u32).clamp(x0 + 1, frame_w) - x0;
    let h = (((rect.y + rect.h) * scale).round() as u32).clamp(y0 + 1, frame_h) - y0;
    let mut crop = Vec::with_capacity((w * h * 4) as usize);
    for row in y0..y0 + h {
        let start = ((row * frame_w + x0) * 4) as usize;
        crop.extend_from_slice(&display.rgba[start..start + (w * 4) as usize]);
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::from_vec(crop, resvg::tiny_skia::IntSize::from_wh(w, h)?)?;
    if !annotations.is_empty() {
        let svg = svg(w, h, scale, rect, annotations);
        let mut options = resvg::usvg::Options::default();
        for font in crate::fonts::annotation_fonts() {
            options.fontdb_mut().load_font_data(font.to_vec());
        }
        options.font_family = "IBM Plex Sans".to_string();
        match resvg::usvg::Tree::from_str(&svg, &options) {
            Ok(tree) => resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut()),
            Err(err) => eprintln!("screenshot: cannot draw the annotations: {err}"),
        }
    }
    let picture = image::RgbaImage::from_raw(w, h, pixmap.take())?;
    let mut bytes = Vec::new();
    picture.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).ok()?;
    Some(bytes)
}

/// The annotations as SVG over a picture of `w` × `h` device pixels: drawn in points, scaled.
fn svg(w: u32, h: u32, scale: f32, rect: Rect, annotations: &[Annotation]) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><g transform="scale({scale}) translate({} {})">"#,
        -rect.x, -rect.y
    );
    for annotation in annotations {
        let color = color_hex(annotation.color);
        let halo = halo_hex(annotation.color);
        let width = annotation.line_width();
        match &annotation.shape {
            Shape::Mosaic { cells, size } => {
                for cell in cells {
                    out += &format!(r#"<rect x="{}" y="{}" width="{size}" height="{size}" fill="{}"/>"#, cell.x, cell.y, cell.color);
                }
            }
            Shape::Box { rect: b, ellipse } => {
                let shape = |stroke: &str, stroke_width: f32| {
                    if *ellipse {
                        format!(
                            r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="none" stroke="{stroke}" stroke-width="{stroke_width}"/>"#,
                            b.x + b.w / 2.0,
                            b.y + b.h / 2.0,
                            b.w / 2.0,
                            b.h / 2.0
                        )
                    } else {
                        format!(
                            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{stroke}" stroke-width="{stroke_width}"/>"#,
                            b.x, b.y, b.w, b.h
                        )
                    }
                };
                out += &shape(&halo, width + 2.0);
                out += &shape(&color, width);
            }
            Shape::Arrow { from, to } => {
                let (shaft, head) = super::arrow_paths(*from, *to, annotation.size);
                for (stroke, stroke_width) in [(&halo, width + 2.0), (&color, width)] {
                    out += &format!(
                        r#"<path d="{shaft}" fill="none" stroke="{stroke}" stroke-width="{stroke_width}" stroke-linecap="round"/><path d="{head}" fill="{stroke}" stroke="{stroke}" stroke-width="{}" stroke-linejoin="round"/>"#,
                        stroke_width - width
                    );
                }
            }
            Shape::Pen { points } => {
                let path = super::polyline(points);
                for (stroke, stroke_width) in [(&halo, width + 2.0), (&color, width)] {
                    out += &format!(
                        r#"<path d="{path}" fill="none" stroke="{stroke}" stroke-width="{stroke_width}" stroke-linecap="round" stroke-linejoin="round"/>"#
                    );
                }
            }
            Shape::Text { at, text } => {
                let size = annotation.font_size();
                out += &format!(
                    r#"<text x="{}" y="{}" font-family="IBM Plex Sans, Noto Sans SC" font-weight="600" font-size="{size}" paint-order="stroke" stroke="{halo}" stroke-width="2" stroke-linejoin="round" fill="{color}">{}</text>"#,
                    at.0,
                    at.1 + size * 0.95,
                    escape(text)
                );
            }
        }
    }
    out += "</g></svg>";
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
