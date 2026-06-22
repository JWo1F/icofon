//! Turning an SVG file into a single closed outline expressed in font units.

use std::path::Path;

use anyhow::{Context, Result, bail};
use kurbo::{BezPath, CubicBez, PathEl, Point, Shape};
use usvg::tiny_skia_path::{self, PathSegment};

use crate::font::{ASCENDER, UNITS_PER_EM};

/// How far a quadratic approximation may stray from the original cubic, in font
/// units. A thousandth of the em is far below what any rasterizer can show.
const CUBIC_TO_QUAD_ACCURACY: f64 = 0.2;

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
    let data = normalize_current_color(data);
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .with_context(|| format!("parsing {source}"))?;

    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        bail!("{source} has an empty viewBox");
    }

    let mut drawn = Vec::new();
    let mut found = Findings::default();
    collect(tree.root(), &mut drawn, &mut found);

    let scale = f64::from(UNITS_PER_EM) / f64::from(size.height());
    let mut path = BezPath::new();
    for filled in &drawn {
        let mut piece = BezPath::new();
        append_scaled(&mut piece, &filled.path, scale);
        if filled.even_odd {
            piece = to_nonzero_winding(&piece);
        }
        if filled.background {
            // Shapes are visited in paint order, so what a white shape means is
            // decided by what is already under it.
            if let Some(hole) = knocked_out_of(&piece, &path) {
                path.extend(hole);
            }
            continue;
        }
        path.extend(piece);
    }

    let path = cubics_to_quads(&path);
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
    })
}

/// One filled shape pulled out of the SVG, in paint order.
struct Filled {
    path: tiny_skia_path::Path,
    even_odd: bool,
    /// White shapes are not ink; what they mean depends on what is under them.
    background: bool,
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

fn collect(group: &usvg::Group, out: &mut Vec<Filled>, found: &mut Findings) {
    if group.mask().is_some() {
        found.masked = true;
    }

    for node in group.children() {
        match node {
            usvg::Node::Group(child) => collect(child, out, found),
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
                        out.push(Filled {
                            path: p,
                            even_odd: fill.rule() == usvg::FillRule::EvenOdd,
                            background: is_background(fill.paint()),
                        });
                    }
                }
                if let Some(stroke) = path.stroke()
                    && let Some(p) =
                        stroke_outline(path.data(), stroke).and_then(|p| p.transform(transform))
                {
                    // A stroke outline is always a non-zero shape.
                    out.push(Filled {
                        path: p,
                        even_odd: false,
                        background: is_background(stroke.paint()),
                    });
                }
            }
            usvg::Node::Image(_) => found.raster = true,
            // With text support compiled out, text has no outline to contribute.
            _ => {}
        }
    }
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

/// Whether a paint is white enough to mean "background" rather than ink.
///
/// A glyph has no colour, only ink and its absence, so a white shape cannot be
/// drawn — painting it would put ink exactly where the artwork wanted none. The
/// clearest case is a badge built as a white panel with a dark outline and dark
/// lettering on top: filling the panel buries the whole design under a solid
/// block. Treating white as nothing keeps the parts that carry the shape.
///
/// Order is deliberately ignored. A white shape painted *over* darker artwork is
/// a knock-out and would ideally be subtracted, but every shape ends up in one
/// non-zero path where drawing order is gone, so subtracting would also eat
/// artwork that merely sits behind it. Dropping is never worse than the solid
/// blob that painting produces, and often much better.
fn is_background(paint: &usvg::Paint) -> bool {
    const NEAR_WHITE: u8 = 0xF0;
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
    fn a_white_shape_over_nothing_is_simply_dropped() {
        let o = outline(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect width="24" height="24" fill="white"/>
                </svg>"##,
        );
        assert_eq!(o.problem, Some(Problem::Empty));
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
