//! A frame as SVG: previews of the window on Linux (rsvg-convert makes a PNG) and a
//! readable record in tests. Text width is estimated, so an ellipsis may fall a few
//! characters off from DirectWrite's.
use crate::draw::{Align, Color, Font, Item};
use crate::view::Frame;

fn col(c: Color) -> String {
    format!("rgba({},{},{},{:.3})", c.r(), c.g(), c.b(), c.a() as f32 / 255.0)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn fit(text: &str, size: f32, font: Font, width: f32) -> String {
    let per = size * if font == Font::Mono { 0.6 } else { 0.55 };
    let max = (width / per).floor().max(1.0) as usize;
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

pub fn to_svg(f: &Frame, w: f32, h: f32) -> String {
    let mut o = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n");
    let mut clips = 0;
    let mut open = 0;
    for it in &f.items {
        match it {
            Item::Box { r, radius, fill, stroke } => {
                o += &format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\" fill=\"{}\"{}/>\n",
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    col(*fill),
                    stroke.map(|s| format!(" stroke=\"{}\" stroke-width=\"1\"", col(s))).unwrap_or_default()
                );
            }
            Item::Text { r, text, size, weight, color, align, font } => {
                let (x, anchor) = match align {
                    Align::Left => (r.x, "start"),
                    Align::Center => (r.x + r.w / 2.0, "middle"),
                    Align::Right => (r.right(), "end"),
                };
                let family = if *font == Font::Mono { "Consolas, DejaVu Sans Mono, monospace" } else { "Segoe UI, DejaVu Sans, sans-serif" };
                o += &format!(
                    "<text x=\"{x}\" y=\"{}\" font-family=\"{family}\" font-size=\"{size}\" font-weight=\"{weight}\" fill=\"{}\" text-anchor=\"{anchor}\" dominant-baseline=\"central\">{}</text>\n",
                    r.y + r.h / 2.0,
                    col(*color),
                    esc(&fit(text, *size, *font, r.w))
                );
            }
            Item::Dot { cx, cy, radius, color, glow } => {
                if *glow {
                    o += &format!("<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{}\" fill=\"{}\" opacity=\"0.35\"/>\n", radius * 1.8, col(*color));
                }
                o += &format!("<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{radius}\" fill=\"{}\"/>\n", col(*color));
            }
            Item::Ring { cx, cy, radius, width, color } => {
                o += &format!("<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{radius}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\"/>\n", col(*color));
            }
            Item::Clip { r } => {
                clips += 1;
                open += 1;
                o += &format!(
                    "<clipPath id=\"c{clips}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath><g clip-path=\"url(#c{clips})\">\n",
                    r.x, r.y, r.w, r.h
                );
            }
            Item::Unclip => {
                if open > 0 {
                    open -= 1;
                    o += "</g>\n";
                }
            }
        }
    }
    for _ in 0..open {
        o += "</g>\n";
    }
    o + "</svg>\n"
}
