//! Turning an SVG file into a single closed outline expressed in font units.

use std::path::Path;

use anyhow::{Context, Result, bail};
use kurbo::{BezPath, CubicBez, PathEl, Point, Shape};
use usvg::tiny_skia_path::{self, PathSegment};

use crate::font::{ASCENDER, UNITS_PER_EM};

/// How far a quadratic approximation may stray from the original cubic, in font
/// units. A thousandth of the em is far below what any rasterizer can show.
const CUBIC_TO_QUAD_ACCURACY: f64 = 0.2;

/// The colour a layer is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerPaint {
    /// Drawn with `currentColor`, so it follows the CSS `color` of whatever the
    /// icon sits in — the same behaviour a plain monochrome glyph has.
    Foreground,
    /// Drawn in a colour the artwork names, which is kept as drawn.
    Fixed { r: u8, g: u8, b: u8, a: u8 },
}

/// How an icon is coloured, which decides both how it is compiled and how it
/// behaves on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colouring {
    /// Drawn entirely in `currentColor`. Follows the CSS `color` completely.
    Foreground,
    /// Drawn in one colour the artwork names. Still compiled as a plain glyph,
    /// so it follows the CSS `color` too — the colour it was drawn in is not
    /// kept, because a single colour carries no relationship worth pinning.
    Single { r: u8, g: u8, b: u8 },
    /// Uses two or more colours, so it is compiled as COLR layers and keeps
    /// them. Only its `currentColor` layers follow the CSS `color`.
    Multi,
}

/// One colour's worth of an icon. Layers are in paint order, bottom first.
#[derive(Debug)]
pub struct Layer {
    pub path: BezPath,
    pub paint: LayerPaint,
}

/// One icon's artwork, already scaled and flipped into the font's coordinate
/// system: y points up, the baseline is y = 0.
#[derive(Debug)]
pub struct Outline {
    pub path: BezPath,
    /// Horizontal advance for the glyph, in font units.
    pub advance: u16,
    /// Set when the SVG cannot be faithfully turned into a glyph, so the caller
    /// can report it instead of shipping a blank or blacked-out icon.
    pub problem: Option<Problem>,
    /// The icon split by colour, bottom layer first.
    ///
    /// Empty unless `colouring` is [`Colouring::Multi`]: everything else is a
    /// plain glyph that follows the CSS `color`.
    pub layers: Vec<Layer>,
    /// How the artwork was coloured, so the preview can say so and filter on it.
    pub colouring: Colouring,
}

/// A reason an SVG cannot become a glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// The artwork is a bitmap wrapped in an SVG — typically a PNG pasted out
    /// of a design tool. A glyph is an outline, so there is nothing to trace.
    RasterImage,
    /// The artwork is shaped by a mask, which decides per-pixel how much of
    /// each shape survives. An outline has no such control, so ignoring the
    /// mask would draw shapes the design never showed.
    Masked,
    /// Nothing drawable came out of the file.
    Empty,
}

impl Problem {
    pub fn explain(self) -> &'static str {
        match self {
            Problem::RasterImage => {
                "the artwork is a bitmap embedded in the SVG, and a glyph can only be an outline; \
                 re-export it as vector paths"
            }
            Problem::Masked => {
                "the design relies on a <mask>, which cannot be expressed as an outline; \
                 flatten the mask in your design tool and re-export"
            }
            Problem::Empty => {
                "nothing drawable in the file; it may use an SVG feature a font cannot represent"
            }
        }
    }
}

/// Parse `file` and flatten everything drawable in it into a single outline.
///
/// The SVG's viewBox height is mapped onto the full em, so an icon drawn edge
/// to edge in its viewBox comes out exactly one em tall. Width is preserved at
/// the same scale and becomes the glyph's advance, so non-square icons keep
/// their proportions.
pub fn load(file: &Path) -> Result<Outline> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    parse(&data, &file.display().to_string())
}

/// A colour no icon would choose, used to mark `currentColor` so it can still
/// be told apart after usvg resolves it to a concrete value.
const FOREGROUND_SENTINEL: usvg::Color = usvg::Color {
    red: 0x01,
    green: 0x02,
    blue: 0x03,
};

/// Give the root `<svg>` a `color` so `currentColor` resolves to the sentinel.
///
/// usvg resolves `currentColor` while parsing and keeps no record of it, so
/// without this a shape drawn with `currentColor` is indistinguishable from one
/// drawn in black — and the two want opposite treatment, one following the CSS
/// `color` and the other staying as drawn.
///
/// An `svg` element that already sets `color` is left alone: there
/// `currentColor` names a colour the artwork chose, which is not the foreground.
fn mark_current_color(data: Vec<u8>) -> Vec<u8> {
    let Some(open) = find(&data, b"<svg") else {
        return data;
    };
    let Some(close) = data[open..]
        .iter()
        .position(|b| *b == b'>')
        .map(|i| open + i)
    else {
        return data;
    };
    let tag = &data[open..close];
    if find(tag, b"color=").is_some_and(|at| {
        // `fill-color=` and the like do not count; the attribute must stand alone.
        at == 0 || !tag[at - 1].is_ascii_alphanumeric() && tag[at - 1] != b'-'
    }) {
        return data;
    }

    let mut out = Vec::with_capacity(data.len() + 24);
    out.extend_from_slice(&data[..open + 4]);
    out.extend_from_slice(
        format!(
            " color=\"#{:02x}{:02x}{:02x}\"",
            FOREGROUND_SENTINEL.red, FOREGROUND_SENTINEL.green, FOREGROUND_SENTINEL.blue
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data[open + 4..]);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Rewrite `currentcolor` to the spelling usvg accepts.
///
/// SVG and CSS keywords are case-insensitive, so `currentcolor` is valid and
/// design tools do emit it, but usvg only matches `currentColor` and silently
/// drops the paint — which turns a stroke-drawn icon into an empty glyph.
fn normalize_current_color(data: &[u8]) -> Vec<u8> {
    const KEYWORD: &[u8] = b"currentcolor";
    const CANONICAL: &[u8] = b"currentColor";

    let lower: Vec<u8> = data.to_ascii_lowercase();
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if lower[i..].starts_with(KEYWORD) {
            let end = i + KEYWORD.len();
            // Only a standalone keyword, never part of a longer identifier.
            let boundary = |b: u8| !b.is_ascii_alphanumeric() && b != b'-' && b != b'_';
            let before_ok = i == 0 || boundary(data[i - 1]);
            let after_ok = end >= data.len() || boundary(data[end]);
            if before_ok && after_ok {
                out.extend_from_slice(CANONICAL);
                i = end;
                continue;
            }
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

/// The body of [`load`], split out so it can be exercised without touching disk.
pub(crate) fn parse(data: &[u8], source: &str) -> Result<Outline> {
    let data = mark_current_color(normalize_current_color(data));
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .with_context(|| format!("parsing {source}"))?;

    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        bail!("{source} has an empty viewBox");
    }

    let mut drawn = Vec::new();
    let mut found = Findings::default();
    collect(tree.root(), 1.0, &mut drawn, &mut found);

    // Paper only means anything next to ink. An icon drawn entirely in white,
    // or entirely as a faint wash, is simply a light icon and must still be
    // drawn — so the paper rule only applies when something else is full
    // strength.
    let has_ink = drawn.iter().any(|filled| !filled.background);
    let scale = f64::from(UNITS_PER_EM) / f64::from(size.height());
    let mut path = BezPath::new();
    for filled in &drawn {
        let mut piece = BezPath::new();
        append_scaled(&mut piece, &filled.path, scale);
        if filled.even_odd {
            piece = to_nonzero_winding(&piece);
        }
        if filled.background && has_ink {
            // Shapes are visited in paint order, so what a paper shape means is
            // decided by what is already under it.
            if let Some(hole) = knocked_out_of(&piece, &path) {
                path.extend(hole);
            }
            continue;
        }
        path.extend(piece);
    }

    let path = cubics_to_quads(&path);
    let colouring = classify(&drawn);
    let layers = if colouring == Colouring::Multi {
        build_layers(&drawn, scale)
    } else {
        Vec::new()
    };
    let problem = if found.raster {
        Some(Problem::RasterImage)
    } else if found.masked {
        Some(Problem::Masked)
    } else if path.is_empty() {
        Some(Problem::Empty)
    } else {
        None
    };

    Ok(Outline {
        path,
        advance: (f64::from(size.width()) * scale).round().max(0.0) as u16,
        problem,
        layers,
        colouring,
    })
}

/// One filled shape pulled out of the SVG, in paint order.
struct Filled {
    path: tiny_skia_path::Path,
    even_odd: bool,
    /// Paper shapes are not ink; what they mean depends on what is under them.
    /// Only the flattened outline cares — a colour layer keeps its own colour.
    background: bool,
    paint: LayerPaint,
}

/// Walk the usvg tree and collect every visible path, in absolute coordinates.
///
/// Fills contribute their own outline; strokes are converted to an outline of
/// their own so that stroke-drawn icon sets (Feather and friends) survive the
/// trip into a font, which only knows how to fill.
/// What the walk noticed that stops the artwork becoming a faithful glyph.
#[derive(Default)]
struct Findings {
    raster: bool,
    masked: bool,
}

fn collect(group: &usvg::Group, alpha: f32, out: &mut Vec<Filled>, found: &mut Findings) {
    if group.mask().is_some() {
        found.masked = true;
    }

    for node in group.children() {
        match node {
            usvg::Node::Group(child) => collect(child, alpha * child.opacity().get(), out, found),
            usvg::Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }
                let transform = path.abs_transform();
                if let Some(fill) = path.fill() {
                    // A pattern fill means the shape is a window onto other
                    // content — in practice a pasted bitmap. Tracing its outline
                    // would emit a solid box where the picture should be.
                    if matches!(fill.paint(), usvg::Paint::Pattern(_)) {
                        found.raster = true;
                    } else if let Some(p) = path.data().clone().transform(transform) {
                        let alpha = alpha * fill.opacity().get();
                        out.push(Filled {
                            path: p,
                            even_odd: fill.rule() == usvg::FillRule::EvenOdd,
                            background: is_background(fill.paint(), alpha),
                            paint: layer_paint(fill.paint(), alpha),
                        });
                    }
                }
                if let Some(stroke) = path.stroke()
                    && let Some(p) =
                        stroke_outline(path.data(), stroke).and_then(|p| p.transform(transform))
                {
                    // A stroke outline is always a non-zero shape.
                    let alpha = alpha * stroke.opacity().get();
                    out.push(Filled {
                        path: p,
                        even_odd: false,
                        background: is_background(stroke.paint(), alpha),
                        paint: layer_paint(stroke.paint(), alpha),
                    });
                }
            }
            usvg::Node::Image(_) => found.raster = true,
            // With text support compiled out, text has no outline to contribute.
            _ => {}
        }
    }
}

/// Decide how an icon is coloured.
///
/// Colour is only worth keeping when there is more than one of it. What colour
/// buys is the *relationship* between colours, which flattening destroys: the
/// white lettering on a dark badge, the three panels of a card logo. A single
/// flat colour has no such relationship — pinning it would only take away the
/// ability to recolour the icon, and an icon frozen in a mid grey disappears
/// against a dark background.
fn classify(drawn: &[Filled]) -> Colouring {
    let mut seen: Vec<LayerPaint> = Vec::new();
    for filled in drawn {
        if !seen.contains(&filled.paint) {
            seen.push(filled.paint);
        }
    }
    match seen.as_slice() {
        [] | [LayerPaint::Foreground] => Colouring::Foreground,
        [LayerPaint::Fixed { r, g, b, .. }] => Colouring::Single {
            r: *r,
            g: *g,
            b: *b,
        },
        _ => Colouring::Multi,
    }
}

/// Split the artwork into one layer per run of shapes sharing a colour.
///
/// Colour layers are built from the artwork as drawn. None of the paper rules
/// apply here — with real colours available, white is white again and a wash is
/// a wash, so a badge keeps its white panel instead of having it dropped.
fn build_layers(drawn: &[Filled], scale: f64) -> Vec<Layer> {
    let mut layers: Vec<Layer> = Vec::new();
    for filled in drawn {
        let mut piece = BezPath::new();
        append_scaled(&mut piece, &filled.path, scale);
        if filled.even_odd {
            piece = to_nonzero_winding(&piece);
        }
        let piece = cubics_to_quads(&piece);
        if piece.is_empty() {
            continue;
        }

        // Consecutive shapes of one colour are a single layer.
        match layers.last_mut() {
            Some(last) if last.paint == filled.paint => last.path.extend(piece),
            _ => layers.push(Layer {
                path: piece,
                paint: filled.paint,
            }),
        }
    }
    layers
}

/// Turn a white shape into a hole in `ink`, if that is what it was doing.
///
/// A white shape means one of two things, and only what lies beneath it tells
/// them apart. Over existing ink it is a knock-out — the tick cut out of a
/// filled circle, the wordmark cut out of a brand panel — and becomes a hole.
/// Over nothing it is background, like the white panel a badge is built on, and
/// is dropped: painting it would bury the design under a solid block.
///
/// Returns the contour to append, wound against the ink beneath so that
/// non-zero filling cancels it out.
fn knocked_out_of(white: &BezPath, ink: &BezPath) -> Option<BezPath> {
    let (over, under) = (white.bounding_box(), ink.bounding_box());
    // Require containment. A shape only partly over the ink would leave a
    // reversed contour sitting in open space, which non-zero filling paints in
    // rather than removes.
    if under.width() <= 0.0 || !under.contains_rect(over) {
        return None;
    }

    let winding = ink.winding(over.center());
    if winding == 0 {
        return None;
    }

    // Cancel the ink: the hole must wind opposite to what is under it.
    let wanted = -winding.signum();
    let current = if white.area() > 0.0 { 1 } else { -1 };
    Some(if current == wanted {
        white.clone()
    } else {
        white.reverse_subpaths()
    })
}

/// The colour a shape contributes to the colour table.
///
/// A gradient is reduced to its first stop: a glyph layer is one flat colour, so
/// the choice is which single colour best stands for the ramp, and the colour it
/// starts from is the most predictable answer.
fn layer_paint(paint: &usvg::Paint, alpha: f32) -> LayerPaint {
    let opaque = |color: usvg::Color| LayerPaint::Fixed {
        r: color.red,
        g: color.green,
        b: color.blue,
        a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    };
    match paint {
        usvg::Paint::Color(color) if *color == FOREGROUND_SENTINEL => LayerPaint::Foreground,
        usvg::Paint::Color(color) => opaque(*color),
        usvg::Paint::LinearGradient(gradient) => gradient
            .stops()
            .first()
            .map_or(LayerPaint::Foreground, |stop| opaque(stop.color())),
        usvg::Paint::RadialGradient(gradient) => gradient
            .stops()
            .first()
            .map_or(LayerPaint::Foreground, |stop| opaque(stop.color())),
        usvg::Paint::Pattern(_) => LayerPaint::Foreground,
    }
}

/// Whether a paint reads as paper rather than ink.
///
/// A glyph is ink or nothing — it has neither colour nor opacity — so two kinds
/// of paint cannot be drawn as themselves:
///
/// * **White**, which would put ink exactly where the artwork wanted none. A
///   badge built as a white panel with dark lettering would be buried under a
///   solid block.
/// * **A faint wash**, a shape at low opacity used as a tint behind the real
///   mark. Drawn at full strength it swallows whatever it was sitting behind.
///
/// Both are handled the same way by the caller: over existing ink they become a
/// knock-out, and over nothing they are dropped.
fn is_background(paint: &usvg::Paint, alpha: f32) -> bool {
    // Below half strength a shape is a tint, not the mark itself.
    const FAINT: f32 = 0.5;
    const NEAR_WHITE: u8 = 0xF0;

    if alpha < FAINT {
        return true;
    }
    match paint {
        usvg::Paint::Color(color) => {
            color.red >= NEAR_WHITE && color.green >= NEAR_WHITE && color.blue >= NEAR_WHITE
        }
        _ => false,
    }
}

fn stroke_outline(
    path: &tiny_skia_path::Path,
    stroke: &usvg::Stroke,
) -> Option<tiny_skia_path::Path> {
    let props = tiny_skia_path::Stroke {
        width: stroke.width().get(),
        miter_limit: stroke.miterlimit().get(),
        line_cap: match stroke.linecap() {
            usvg::LineCap::Butt => tiny_skia_path::LineCap::Butt,
            usvg::LineCap::Round => tiny_skia_path::LineCap::Round,
            usvg::LineCap::Square => tiny_skia_path::LineCap::Square,
        },
        line_join: match stroke.linejoin() {
            usvg::LineJoin::Miter => tiny_skia_path::LineJoin::Miter,
            usvg::LineJoin::MiterClip => tiny_skia_path::LineJoin::MiterClip,
            usvg::LineJoin::Round => tiny_skia_path::LineJoin::Round,
            usvg::LineJoin::Bevel => tiny_skia_path::LineJoin::Bevel,
        },
        dash: None,
    };
    path.stroke(&props, 1.0)
}

/// Copy an SVG path into `out`, scaling it and flipping the y axis on the way.
///
/// Every contour is closed explicitly: glyf closes contours implicitly anyway,
/// and being explicit keeps the conversion below from having to guess.
fn append_scaled(out: &mut BezPath, src: &tiny_skia_path::Path, scale: f64) {
    let point = |p: tiny_skia_path::Point| {
        Point::new(
            f64::from(p.x) * scale,
            f64::from(ASCENDER) - f64::from(p.y) * scale,
        )
    };

    let mut open = false;
    for segment in src.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                if open {
                    out.close_path();
                }
                out.move_to(point(p));
                open = true;
            }
            PathSegment::LineTo(p) => out.line_to(point(p)),
            PathSegment::QuadTo(c, p) => out.quad_to(point(c), point(p)),
            PathSegment::CubicTo(c1, c2, p) => out.curve_to(point(c1), point(c2), point(p)),
            PathSegment::Close => {
                out.close_path();
                open = false;
            }
        }
    }
    if open {
        out.close_path();
    }
}

/// Re-orient the contours of an even-odd path so that filling it with the
/// non-zero rule gives the same result.
///
/// TrueType only knows non-zero winding, so an even-odd path whose hole happens
/// to wind the same way as its outer contour would otherwise come out solid. A
/// contour nested an odd number of deep is a hole and must wind against its
/// container; one nested an even number of deep is solid and must wind with the
/// outermost contours.
fn to_nonzero_winding(path: &BezPath) -> BezPath {
    let contours = split_contours(path);
    if contours.len() < 2 {
        return path.clone();
    }

    let mut out = BezPath::new();
    for (index, contour) in contours.iter().enumerate() {
        let Some(probe) = first_point(contour) else {
            continue;
        };
        let depth = contours
            .iter()
            .enumerate()
            .filter(|(other_index, other)| *other_index != index && other.contains(probe))
            .count();

        // Contours at even depth all share one orientation, odd depth the other.
        if (depth % 2 == 0) == (contour.area() > 0.0) {
            out.extend(contour.iter());
        } else {
            out.extend(contour.reverse_subpaths());
        }
    }
    out
}

/// Split a path into one `BezPath` per contour.
fn split_contours(path: &BezPath) -> Vec<BezPath> {
    let mut contours = Vec::new();
    let mut current = BezPath::new();
    for element in path.elements() {
        if matches!(element, PathEl::MoveTo(_)) && !current.is_empty() {
            contours.push(std::mem::take(&mut current));
        }
        current.push(*element);
    }
    if !current.is_empty() {
        contours.push(current);
    }
    contours
}

fn first_point(contour: &BezPath) -> Option<Point> {
    match contour.elements().first() {
        Some(PathEl::MoveTo(p)) => Some(*p),
        _ => None,
    }
}

/// Replace every cubic segment with a run of quadratics, which is all the glyf
/// table can store.
fn cubics_to_quads(src: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut current = Point::ZERO;
    let mut subpath_start = Point::ZERO;

    for element in src.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                out.move_to(p);
                current = p;
                subpath_start = p;
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                current = p;
            }
            PathEl::QuadTo(c, p) => {
                out.quad_to(c, p);
                current = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                let cubic = CubicBez::new(current, c1, c2, p);
                for (_, _, quad) in cubic.to_quads(CUBIC_TO_QUAD_ACCURACY) {
                    out.quad_to(quad.p1, quad.p2);
                }
                current = p;
            }
            PathEl::ClosePath => {
                out.close_path();
                current = subpath_start;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn outline(svg: &str) -> Outline {
        parse(svg.as_bytes(), "test.svg").unwrap()
    }

    #[test]
    fn viewbox_height_maps_onto_the_em() {
        // A square filling its whole viewBox should fill the whole em box.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                 <rect width="24" height="24" fill="#000"/>
               </svg>"##,
        );
        let bbox = o.path.bounding_box();
        assert_eq!(o.advance, UNITS_PER_EM);
        assert!((bbox.x0 - 0.0).abs() < 0.5, "x0 = {}", bbox.x0);
        assert!((bbox.x1 - 1000.0).abs() < 0.5, "x1 = {}", bbox.x1);
        assert!((bbox.y0 - -200.0).abs() < 0.5, "y0 = {}", bbox.y0);
        assert!((bbox.y1 - 800.0).abs() < 0.5, "y1 = {}", bbox.y1);
    }

    #[test]
    fn width_becomes_the_advance() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 24">
                 <rect width="48" height="24" fill="#000"/>
               </svg>"##,
        );
        assert_eq!(o.advance, 2 * UNITS_PER_EM);
    }

    #[test]
    fn strokes_become_outlines() {
        // Nothing is filled here, so without stroke conversion the glyph would
        // come out blank.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                 <line x1="4" y1="12" x2="20" y2="12" fill="none" stroke="#000" stroke-width="2"/>
               </svg>"##,
        );
        assert!(!o.path.is_empty());
        let bbox = o.path.bounding_box();
        assert!(
            bbox.height() > 0.0,
            "a stroked line should enclose some area"
        );
    }

    #[test]
    fn shapes_and_transforms_are_honoured() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                 <g transform="translate(12 0)"><circle cx="6" cy="12" r="6" fill="#000"/></g>
               </svg>"##,
        );
        let bbox = o.path.bounding_box();
        // The circle sits in the right half of the viewBox after the translate.
        assert!(bbox.x0 > 490.0, "x0 = {}", bbox.x0);
    }

    /// A ring drawn as two same-winding circles: only the fill rule tells the
    /// hole apart from the outer disc.
    const EVEN_ODD_RING: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
             <path fill-rule="evenodd" fill="#000"
                   d="M12 2A10 10 0 1 1 11.99 2Z M12 6A6 6 0 1 1 11.99 6Z"/>
           </svg>"##;

    #[test]
    fn even_odd_holes_survive_as_non_zero() {
        let o = outline(EVEN_ODD_RING);
        // The em centre corresponds to the middle of the viewBox.
        assert!(
            !o.path.contains(Point::new(500.0, 300.0)),
            "the hole should be empty under non-zero winding"
        );
        assert!(
            o.path.contains(Point::new(500.0, 630.0)),
            "the ring itself should be filled"
        );
    }

    #[test]
    fn non_zero_paths_are_left_alone() {
        // Same ring, but already drawn with opposing windings.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path fill="#000" d="M12 2A10 10 0 1 0 12 22A10 10 0 1 0 12 2Z
                                       M12 6A6 6 0 1 1 12 18A6 6 0 1 1 12 6Z"/>
                </svg>"##,
        );
        assert!(!o.path.contains(Point::new(500.0, 300.0)));
        assert!(o.path.contains(Point::new(500.0, 630.0)));
    }

    #[test]
    fn a_white_panel_is_background_not_ink() {
        // A badge built as a white panel with a dark outline: painting the
        // panel would bury the design under a solid block.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="2" y="2" width="20" height="20" fill="white"
                        stroke="#000" stroke-width="2"/>
                </svg>"##,
        );
        assert!(!o.path.is_empty(), "the dark outline still draws");
        assert!(
            !o.path.contains(Point::new(500.0, 300.0)),
            "the middle of the panel must stay empty"
        );
        assert_eq!(o.problem, None);
    }

    #[test]
    fn white_over_ink_is_knocked_out() {
        // A tick cut out of a filled circle: the white shape is a hole, not a
        // block of ink, so the middle must be empty and the ring solid.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <circle cx="12" cy="12" r="10" fill="#000"/>
                  <circle cx="12" cy="12" r="5" fill="white"/>
                </svg>"##,
        );
        assert!(
            !o.path.contains(Point::new(500.0, 300.0)),
            "the knocked-out middle must be empty"
        );
        assert!(
            o.path.contains(Point::new(500.0, 612.0)),
            "the ring around it must still be ink"
        );
    }

    #[test]
    fn a_white_shape_over_nothing_is_dropped_only_if_there_is_ink() {
        // White beside ink is the panel a badge sits on, and is dropped.
        let with_ink = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect width="24" height="24" fill="white"/>
                  <rect x="10" y="10" width="4" height="4" fill="#000"/>
                </svg>"##,
        );
        assert!(
            !with_ink.path.contains(Point::new(100.0, 700.0)),
            "the white panel must not be painted"
        );

        // White on its own is not a panel, it is the icon. Dropping it would
        // leave nothing at all.
        let alone = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect width="24" height="24" fill="white"/>
                </svg>"##,
        );
        assert_eq!(alone.problem, None);
        assert!(alone.path.contains(Point::new(500.0, 300.0)));
    }

    #[test]
    fn masked_artwork_is_reported_rather_than_over_drawn() {
        // Ignoring the mask would paint the whole square instead of the
        // quarter the design actually shows.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <mask id="m" style="mask-type:luminance">
                    <rect width="12" height="12" fill="white"/>
                  </mask>
                  <g mask="url(#m)"><rect width="24" height="24" fill="#000"/></g>
                </svg>"##,
        );
        assert_eq!(o.problem, Some(Problem::Masked));
    }

    #[test]
    fn a_bitmap_wrapped_in_an_svg_is_reported() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <image width="24" height="24"
                         href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="/>
                </svg>"##,
        );
        assert_eq!(o.problem, Some(Problem::RasterImage));
    }

    #[test]
    fn a_faint_wash_behind_the_mark_is_not_drawn() {
        // The tinted disc behind this ring would otherwise fill solid and
        // swallow the mark sitting on it.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18">
                  <path opacity="0.2" d="M9 2A7 7 0 1 1 8.99 2Z" fill="#0781B5"/>
                  <path d="M9 3.5A5.5 5.5 0 1 1 8.99 3.5Z" fill="none"
                        stroke="#0781B5" stroke-width="1.5"/>
                </svg>"##,
        );
        assert!(
            !o.path.contains(Point::new(500.0, 300.0)),
            "the middle must stay open, not be filled by the tint"
        );
    }

    #[test]
    fn an_icon_that_is_entirely_faint_is_still_drawn() {
        // Nothing here is full strength, so the wash IS the icon.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect opacity="0.4" width="24" height="24" fill="#AD75B4"/>
                </svg>"##,
        );
        assert_eq!(o.problem, None);
        assert!(o.path.contains(Point::new(500.0, 300.0)));
    }

    #[test]
    fn a_single_colour_icon_stays_a_recolourable_symbol() {
        // One flat colour has no relationship for colour to preserve, and
        // pinning it would stop the icon following the CSS `color`.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M2 2h20v20H2Z" fill="#3F3C43"/>
                  <path d="M6 6h12v4H6Z" fill="#3F3C43"/>
                </svg>"##,
        );
        assert!(o.layers.is_empty());
    }

    #[test]
    fn an_icon_drawn_in_current_color_stays_a_symbol() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M2 2h20v20H2Z" fill="currentColor"/>
                </svg>"##,
        );
        assert!(o.layers.is_empty());
    }

    #[test]
    fn two_colours_become_layers_in_paint_order() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M0 0h24v24H0Z" fill="#FF4F00"/>
                  <path d="M6 6h12v12H6Z" fill="white"/>
                </svg>"##,
        );
        let paints: Vec<_> = o.layers.iter().map(|l| l.paint).collect();
        assert_eq!(
            paints,
            [
                LayerPaint::Fixed {
                    r: 0xFF,
                    g: 0x4F,
                    b: 0x00,
                    a: 255
                },
                LayerPaint::Fixed {
                    r: 0xFF,
                    g: 0xFF,
                    b: 0xFF,
                    a: 255
                },
            ],
            "the panel is the bottom layer and the mark sits on it"
        );
    }

    #[test]
    fn current_color_survives_beside_a_fixed_colour() {
        // The tick follows the CSS colour while the disc stays blue.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <circle cx="12" cy="12" r="10" fill="#0781B5"/>
                  <path d="M7 12l3 3 7-7" fill="none" stroke="currentColor" stroke-width="2"/>
                </svg>"##,
        );
        let paints: Vec<_> = o.layers.iter().map(|l| l.paint).collect();
        assert_eq!(
            paints,
            [
                LayerPaint::Fixed {
                    r: 0x07,
                    g: 0x81,
                    b: 0xB5,
                    a: 255
                },
                LayerPaint::Foreground,
            ]
        );
    }

    #[test]
    fn an_svg_that_sets_its_own_color_keeps_it() {
        // Here `currentColor` names a colour the artwork chose, so it is not
        // the foreground and the icon is a two-colour one.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" color="#FF0000">
                  <path d="M0 0h24v24H0Z" fill="currentColor"/>
                  <path d="M6 6h12v12H6Z" fill="#0000FF"/>
                </svg>"##,
        );
        let paints: Vec<_> = o.layers.iter().map(|l| l.paint).collect();
        assert_eq!(
            paints,
            [
                LayerPaint::Fixed {
                    r: 0xFF,
                    g: 0,
                    b: 0,
                    a: 255
                },
                LayerPaint::Fixed {
                    r: 0,
                    g: 0,
                    b: 0xFF,
                    a: 255
                },
            ]
        );
    }

    #[test]
    fn the_flattened_outline_is_still_built_for_a_colour_icon() {
        // COLR needs a base glyph for renderers without colour support.
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M0 0h24v24H0Z" fill="#FF4F00"/>
                  <path d="M6 6h12v12H6Z" fill="white"/>
                </svg>"##,
        );
        assert!(!o.layers.is_empty());
        assert!(!o.path.is_empty(), "the fallback outline must still exist");
    }

    #[test]
    fn no_cubics_survive() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                 <path d="M2 12 C 2 2, 22 2, 22 12 Z" fill="#000"/>
               </svg>"##,
        );
        assert!(
            !o.path
                .elements()
                .iter()
                .any(|e| matches!(e, PathEl::CurveTo(..))),
            "glyf cannot store cubic segments"
        );
    }
}
