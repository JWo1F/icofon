//! Turning an SVG file into a single closed outline expressed in font units.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use kurbo::{BezPath, CubicBez, Line, PathEl, Point, Rect, Shape};
use usvg::tiny_skia_path::{self, PathSegment};

use crate::config::Color;
use crate::font::{ASCENDER, UNITS_PER_EM};
use crate::region;

/// How far a quadratic approximation may stray from the original cubic, in font
/// units. A thousandth of the em is far below what any rasterizer can show.
const CUBIC_TO_QUAD_ACCURACY: f64 = 0.2;

/// The color a layer is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerPaint {
  /// Drawn with `currentColor`, so it follows the CSS `color` of whatever the
  /// icon sits in — the same behavior a plain monochrome glyph has. `a` is how
  /// much of that color comes through: a duotone icon draws its body in the
  /// text color at a fraction of full strength.
  Foreground { a: u8 },
  /// Drawn in a color the artwork names, which is kept as drawn.
  Fixed { r: u8, g: u8, b: u8, a: u8 },
}

/// How an icon is colored, which is really the question "what can I change?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coloring {
  /// One color, and CSS `color` sets it. Compiled as a plain glyph, whether
  /// the artwork said `currentColor` or named a color — a lone color carries
  /// no relationship worth pinning, so it is left free to change.
  Single,
  /// Several colors, one of which follows CSS `color` while the rest are
  /// fixed: a blue disc with a tick that recolors with the page.
  Mixed,
  /// Several colors, all fixed by the artwork. CSS `color` does nothing.
  Fixed,
}

impl LayerPaint {
  /// Whether the paint follows the CSS `color` rather than naming one itself.
  pub fn follows_text(self) -> bool {
    matches!(self, LayerPaint::Foreground { .. })
  }
}

/// One color's worth of an icon. Layers are in paint order, bottom first.
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
  /// The icon split by color, bottom layer first.
  ///
  /// Empty for [`Coloring::Single`], which is a plain glyph that follows the
  /// CSS `color`.
  pub layers: Vec<Layer>,
  /// How the artwork was colored, so the preview can say so and filter on it.
  pub coloring: Coloring,
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
  /// The file is not SVG a parser will take — truncated, or not SVG at all.
  Unreadable,
  /// The artwork is so much wider than it is tall that its outline runs past
  /// what a glyph can hold. Coordinates are 16-bit and the height is mapped
  /// onto the em, so the width has nowhere to go.
  TooWide,
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
      Problem::Unreadable => {
        "the file could not be read as SVG; it may be truncated or not an SVG at all"
      }
      Problem::TooWide => {
        "the artwork is more than 32 times wider than it is tall, which puts its outline \
                 past the 16-bit coordinates a glyph is stored in; crop it or split it up"
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
pub fn load(file: &Path, color: Color) -> Result<Outline> {
  let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
  // A file the parser will not take is one more icon that cannot become a
  // glyph, and belongs with the others rather than stopping the build where
  // `--on-error skip` cannot reach it.
  Ok(
    parse(&data, &file.display().to_string(), color).unwrap_or_else(|_| Outline {
      path: BezPath::new(),
      advance: 0,
      problem: Some(Problem::Unreadable),
      layers: Vec::new(),
      coloring: Coloring::Single,
    }),
  )
}

/// A color no icon would choose, used to mark `currentColor` so it can still
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
/// `currentColor` names a color the artwork chose, which is not the foreground.
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
pub(crate) fn parse(data: &[u8], source: &str, color: Color) -> Result<Outline> {
  let data = mark_current_color(normalize_current_color(data));
  let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
    .with_context(|| format!("parsing {source}"))?;

  let size = tree.size();
  if size.width() <= 0.0 || size.height() <= 0.0 {
    bail!("{source} has an empty viewBox");
  }

  let mut drawn = Vec::new();
  let mut found = Findings::default();
  collect(tree.root(), 1.0, &[], &mut drawn, &mut found);
  // A shape with no inside paints nothing, and one arrives whenever a line is
  // drawn with only a `stroke`: SVG fills a path black unless told otherwise,
  // so the line turns up carrying a black fill as well. Left in, it costs the
  // icon a color it never showed — enough to stop a one-color icon following
  // the CSS `color`, and to bring its faint layers up to full strength.
  drawn.retain(|filled| encloses_area(&filled.path));

  let scale = f64::from(UNITS_PER_EM) / f64::from(size.height());
  let path = cubics_to_quads(&flatten(&drawn, scale));
  // Glyph coordinates are 16-bit, and the height is mapped onto the em, so a
  // wide enough icon runs off the end of what a glyph can store. Left to
  // itself, write-fonts rounds the overflow away and the artwork is quietly
  // chopped off partway across.
  let too_wide = f64::from(size.width()) * scale > f64::from(i16::MAX);
  let coloring = classify(&drawn, color);
  let layers = match coloring {
    // An icon that follows the CSS `color` throughout is still a layered glyph
    // when it draws that color at two strengths: a plain glyph has no opacity
    // to give the fainter one, and COLR does.
    Coloring::Single if color == Color::Recolor || !washed(&drawn) => Vec::new(),
    _ => build_layers(&drawn, scale),
  };
  let problem = if too_wide {
    Some(Problem::TooWide)
  } else if found.raster {
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
    coloring,
  })
}

/// One filled shape pulled out of the SVG, in paint order.
struct Filled {
  path: tiny_skia_path::Path,
  even_odd: bool,
  /// The clip regions this shape is drawn through, outermost first, in the
  /// same coordinates as `path`. Empty for all but a handful of icons.
  clips: Vec<Arc<tiny_skia_path::Path>>,
  /// Paper and wash shapes are not ink; what they mean depends on what else the
  /// icon draws. Only the flattened outline cares — a color layer has real
  /// color and opacity, and keeps both.
  role: Role,
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

fn collect(
  group: &usvg::Group,
  alpha: f32,
  clips: &[Arc<tiny_skia_path::Path>],
  out: &mut Vec<Filled>,
  found: &mut Findings,
) {
  if group.mask().is_some() {
    found.masked = true;
  }

  // A clip on a group applies to everything under it, so it is carried down
  // rather than applied here: a shape is cut once, when it is scaled, and only
  // if the clip really cuts it.
  let mut clips = clips.to_vec();
  if let Some(clip) = group.clip_path() {
    // A clip is written in the space of the element that refers to it, and
    // usvg leaves that element's transform off the clip's own contents — so a
    // group that flips or scales its children flips and scales its clip too,
    // and the two have to be put back together here.
    clips.extend(clip_regions(clip, group.abs_transform()));
  }

  for node in group.children() {
    match node {
      usvg::Node::Group(child) => collect(child, alpha * child.opacity().get(), &clips, out, found),
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
              clips: clips.clone(),
              role: role_of(fill.paint(), alpha),
              paint: layer_paint(fill.paint(), alpha),
            });
          }
        }
        if let Some(stroke) = path.stroke()
          && let Some(p) = stroke_outline(path.data(), stroke).and_then(|p| p.transform(transform))
        {
          // A stroke outline is always a non-zero shape.
          let alpha = alpha * stroke.opacity().get();
          out.push(Filled {
            path: p,
            even_odd: false,
            clips: clips.clone(),
            role: role_of(stroke.paint(), alpha),
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

/// The shapes a `<clipPath>` keeps, in absolute coordinates.
///
/// A clip is the union of its children, and a clip may itself be clipped, which
/// is an intersection. Both are collected here as a list to intersect the
/// clipped shape with in turn — a single shape, which is what a clip almost
/// always is, then costs nothing to assemble.
fn clip_regions(
  clip: &usvg::ClipPath,
  referrer: tiny_skia_path::Transform,
) -> Vec<Arc<tiny_skia_path::Path>> {
  fn shapes(
    group: &usvg::Group,
    into: tiny_skia_path::Transform,
    out: &mut Vec<tiny_skia_path::Path>,
  ) {
    for node in group.children() {
      match node {
        usvg::Node::Group(child) => shapes(child, into, out),
        usvg::Node::Path(path) => {
          if let Some(p) = path
            .data()
            .clone()
            .transform(into.pre_concat(path.abs_transform()))
          {
            out.push(p);
          }
        }
        _ => {}
      }
    }
  }

  let into = referrer.pre_concat(clip.transform());
  let mut union = Vec::new();
  shapes(clip.root(), into, &mut union);
  // Several shapes in one clip are a union, which is not something a list of
  // intersections can say. Joining them into one path leaves that to the
  // non-zero rule, which is what the clip itself is filled by.
  let joined = if union.len() == 1 {
    union.pop()
  } else {
    let mut builder = tiny_skia_path::PathBuilder::new();
    for shape in &union {
      for segment in shape.segments() {
        match segment {
          PathSegment::MoveTo(p) => builder.move_to(p.x, p.y),
          PathSegment::LineTo(p) => builder.line_to(p.x, p.y),
          PathSegment::QuadTo(c, p) => builder.quad_to(c.x, c.y, p.x, p.y),
          PathSegment::CubicTo(c1, c2, p) => builder.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y),
          PathSegment::Close => builder.close(),
        }
      }
    }
    builder.finish()
  };

  let mut regions: Vec<Arc<tiny_skia_path::Path>> = Vec::new();
  regions.extend(joined.map(Arc::new));
  if let Some(nested) = clip.clip_path() {
    regions.extend(clip_regions(nested, into));
  }
  regions
}

/// Decide how an icon is colored, under the policy the build was given.
///
/// Under [`Color::Keep`] the artwork is taken at its word: a named color is a
/// choice and is kept, and `currentColor` is the one way to ask for something
/// that recolors. Under [`Color::RecolorSingle`] a lone color is read as a default
/// rather than a choice and left free to change — what color buys is the
/// *relationship* between colors (white lettering on a dark badge, the three
/// panels of a card logo), and one flat color has no such relationship. Under
/// [`Color::Recolor`] nothing is kept.
fn classify(drawn: &[Filled], policy: Color) -> Coloring {
  if policy == Color::Recolor {
    return Coloring::Single;
  }
  let mut seen: Vec<LayerPaint> = Vec::new();
  for filled in drawn {
    if !seen.contains(&filled.paint) {
      seen.push(filled.paint);
    }
  }
  if policy == Color::RecolorSingle && seen.len() < 2 {
    return Coloring::Single;
  }
  match (
    seen.iter().any(|paint| paint.follows_text()),
    seen.iter().any(|paint| !paint.follows_text()),
  ) {
    // Nothing fixed to keep: a plain glyph that follows the CSS `color`.
    (_, false) => Coloring::Single,
    (true, true) => Coloring::Mixed,
    (false, true) => Coloring::Fixed,
  }
}

/// Whether the artwork paints `currentColor` at more than one strength.
///
/// A duotone icon built entirely out of `currentColor` — a body at a fraction
/// of full strength with the detail drawn over it — reads as one paint until
/// the strengths are told apart. It follows the CSS `color` throughout, so it
/// is not a color icon; but the two strengths are a relationship, and the only
/// place a font can hold one is a COLR layer.
///
/// A single strength, whatever it is, carries no such relationship: an icon
/// drawn wholly as a wash is just a light icon, and is drawn at full strength
/// like any other — which is what [`Role::Wash`] already says.
fn washed(drawn: &[Filled]) -> bool {
  let mut strengths: Vec<u8> = Vec::new();
  for filled in drawn {
    if let LayerPaint::Foreground { a } = filled.paint
      && !strengths.contains(&a)
    {
      strengths.push(a);
    }
  }
  strengths.len() > 1
}

/// Split the artwork into one layer per run of shapes sharing a color.
///
/// Color layers are built from the artwork as drawn. None of the paper rules
/// apply here — with real colors available, white is white again and a wash is
/// a wash, so a badge keeps its white panel instead of having it dropped.
fn build_layers(drawn: &[Filled], scale: f64) -> Vec<Layer> {
  let mut layers: Vec<Layer> = Vec::new();
  for filled in drawn {
    let Some(piece) = outline_of(filled, scale) else {
      continue;
    };
    let piece = cubics_to_quads(&piece);
    if piece.is_empty() {
      continue;
    }

    // Consecutive shapes of one color are a single layer.
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

/// The color a shape contributes to the color table.
///
/// A gradient is reduced to its first stop: a glyph layer is one flat color, so
/// the choice is which single color best stands for the ramp, and the color it
/// starts from is the most predictable answer.
fn layer_paint(paint: &usvg::Paint, alpha: f32) -> LayerPaint {
  let strength = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
  let opaque = |color: usvg::Color| LayerPaint::Fixed {
    r: color.red,
    g: color.green,
    b: color.blue,
    a: strength,
  };
  let text = LayerPaint::Foreground { a: strength };
  match paint {
    usvg::Paint::Color(color) if *color == FOREGROUND_SENTINEL => text,
    usvg::Paint::Color(color) => opaque(*color),
    usvg::Paint::LinearGradient(gradient) => gradient
      .stops()
      .first()
      .map_or(text, |stop| opaque(stop.color())),
    usvg::Paint::RadialGradient(gradient) => gradient
      .stops()
      .first()
      .map_or(text, |stop| opaque(stop.color())),
    usvg::Paint::Pattern(_) => text,
  }
}

/// What a shape is for, which decides how a glyph can carry it.
///
/// A glyph is ink or nothing — it has neither color nor opacity — so two kinds
/// of paint cannot be drawn as themselves, and they do not want the same
/// treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
  /// Drawn as itself.
  Ink,
  /// **White**, which would put ink exactly where the artwork wanted none. A
  /// badge built as a white panel with dark lettering would be buried under a
  /// solid block. Over ink it is a knock-out; over nothing it is the paper the
  /// design sits on and is dropped.
  Paper,
  /// **A faint wash**: a shape at a fraction of full strength, which duotone
  /// icon sets use for the body an icon is built on. It has no ink of its own,
  /// so a glyph can only make it the whole body or nothing at all — see
  /// [`wash_body`].
  Wash,
}

fn role_of(paint: &usvg::Paint, alpha: f32) -> Role {
  // Below half strength a shape is a wash, not the mark itself.
  const FAINT: f32 = 0.5;
  const NEAR_WHITE: u8 = 0xF0;

  if alpha < FAINT {
    return Role::Wash;
  }
  match paint {
    usvg::Paint::Color(color)
      if color.red >= NEAR_WHITE && color.green >= NEAR_WHITE && color.blue >= NEAR_WHITE =>
    {
      Role::Paper
    }
    _ => Role::Ink,
  }
}

/// Whether a shape has an inside for a fill to land in.
///
/// Measured on the control polygon rather than the curves, which is zero in
/// exactly the same cases: a curve whose control points are in a line is a line.
/// Subpaths are measured one at a time and their areas added as magnitudes, so
/// that two shapes which happen to cancel are not mistaken for one that draws
/// nothing.
fn encloses_area(path: &tiny_skia_path::Path) -> bool {
  // A millionth of a square unit of the source artwork, which no rasterizer
  // could show and no color could be read from.
  const NOTHING: f64 = 1e-6;

  fn cross(from: (f64, f64), to: (f64, f64)) -> f64 {
    from.0 * to.1 - to.0 * from.1
  }
  fn at(p: tiny_skia_path::Point) -> (f64, f64) {
    (f64::from(p.x), f64::from(p.y))
  }

  let mut total = 0.0f64;
  let mut subpath = 0.0f64;
  let mut start = (0.0, 0.0);
  let mut current = start;
  let step = |subpath: &mut f64, current: &mut (f64, f64), to: (f64, f64)| {
    *subpath += cross(*current, to);
    *current = to;
  };

  for segment in path.segments() {
    match segment {
      PathSegment::MoveTo(p) => {
        // Whatever the subpath left open, close before starting the next.
        total += (subpath + cross(current, start)).abs();
        subpath = 0.0;
        start = at(p);
        current = start;
      }
      PathSegment::LineTo(p) => step(&mut subpath, &mut current, at(p)),
      PathSegment::QuadTo(c, p) => {
        step(&mut subpath, &mut current, at(c));
        step(&mut subpath, &mut current, at(p));
      }
      PathSegment::CubicTo(c1, c2, p) => {
        step(&mut subpath, &mut current, at(c1));
        step(&mut subpath, &mut current, at(c2));
        step(&mut subpath, &mut current, at(p));
      }
      PathSegment::Close => {
        step(&mut subpath, &mut current, start);
      }
    }
  }
  total += (subpath + cross(current, start)).abs();

  total / 2.0 > NOTHING
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
    // Dashing is not something the stroker does; it is done below, to the path
    // rather than to its outline.
    dash: None,
  };

  // A dashed stroke is a row of separate marks, and drawing it solid is not a
  // near miss — a dotted rule comes out as a bar. Cutting the path into its
  // dashes first gives the stroker one open subpath per mark, so each comes out
  // as a contour of its own, caps and all.
  let dashed = stroke
    .dasharray()
    .and_then(|pattern| tiny_skia_path::StrokeDash::new(pattern.to_vec(), stroke.dashoffset()))
    .and_then(|dash| path.dash(&dash, 1.0));

  dashed.as_ref().unwrap_or(path).stroke(&props, 1.0)
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

/// Whether the path crosses itself anywhere.
///
/// Only a crossing makes the even-odd rule say something that nesting cannot,
/// so this decides which of the two conversions runs. It is deliberately
/// approximate in the safe direction: the curves are flattened first, so a
/// crossing that is missed only leaves the cheaper conversion in place — the
/// answer it gives is the one that was given before there was a choice.
fn crosses_itself(path: &BezPath) -> bool {
  // Fine enough that a real crossing survives flattening, coarse enough that
  // the count of segments stays workable.
  const FLATNESS: f64 = 0.25;

  let mut edges: Vec<Line> = Vec::new();
  // Where each contour starts in `edges`, so that a contour's own ends are not
  // read as a crossing where they meet.
  let mut contours: Vec<usize> = vec![0];
  let mut current = Point::ZERO;
  let mut start = Point::ZERO;
  kurbo::flatten(path.iter(), FLATNESS, |element| match element {
    PathEl::MoveTo(p) => {
      contours.push(edges.len());
      start = p;
      current = p;
    }
    PathEl::LineTo(p) => {
      edges.push(Line::new(current, p));
      current = p;
    }
    PathEl::ClosePath => {
      if current != start {
        edges.push(Line::new(current, start));
      }
      current = start;
    }
    _ => {}
  });

  let neighbours = |a: usize, b: usize| {
    // Consecutive edges share a point by construction, as do the two ends of a
    // contour, and touching is not crossing.
    if b == a + 1 {
      return true;
    }
    contours
      .windows(2)
      .any(|range| range[0] == a && b + 1 == range[1])
      || (a + 1 == edges.len() && contours.last() == Some(&b))
  };

  let boxes: Vec<Rect> = edges.iter().map(|edge| edge.bounding_box()).collect();
  for a in 0..edges.len() {
    for b in (a + 1)..edges.len() {
      if neighbours(a, b) || !boxes[a].overlaps(boxes[b]) {
        continue;
      }
      if let Some(at) = edges[a].crossing_point(edges[b])
        && strictly_within(edges[a], at)
        && strictly_within(edges[b], at)
      {
        return true;
      }
    }
  }
  false
}

/// Whether a crossing point falls inside a segment rather than at its ends.
fn strictly_within(edge: Line, at: Point) -> bool {
  const ENDS: f64 = 1e-6;
  let length = edge.p1 - edge.p0;
  let along = (at - edge.p0).dot(length) / length.hypot2();
  along > ENDS && along < 1.0 - ENDS
}

/// Flatten every shape into the one outline a plain glyph is.
///
/// A glyph is ink or nothing, so the shapes that are neither have to be read
/// for what they were standing in for — see [`Role`].
fn flatten(drawn: &[Filled], scale: f64) -> BezPath {
  let shapes: Vec<(Role, BezPath)> = drawn
    .iter()
    .filter_map(|filled| Some((filled.role, outline_of(filled, scale)?)))
    .collect();

  // Paper and wash only mean anything next to ink. An icon drawn entirely in
  // white, or entirely as a faint wash, is simply a light icon and must still
  // be drawn.
  let ink: Vec<&BezPath> = shapes
    .iter()
    .filter(|(role, _)| *role == Role::Ink)
    .map(|(_, piece)| piece)
    .collect();
  if ink.is_empty() {
    return shapes
      .into_iter()
      .map(|(_, piece)| piece)
      .fold(BezPath::new(), |mut all, piece| {
        all.extend(piece);
        all
      });
  }

  // A wash that the ink already traces is standing in for a silhouette the icon
  // draws anyway, so it has nothing left to say. The rest are bodies, and a
  // mark drawn inside a body is what tells one body from another: it becomes a
  // hole rather than more ink, which is the only way a glyph can show it.
  let bodies: Vec<&BezPath> = shapes
    .iter()
    .filter(|(role, piece)| *role == Role::Wash && !traced_by(piece, &ink))
    .map(|(_, piece)| piece)
    .collect();

  let mut path = BezPath::new();
  let mut cuts: Vec<BezPath> = Vec::new();
  for (role, piece) in &shapes {
    match role {
      Role::Ink if bodies.iter().any(|body| encloses(body, piece)) => cuts.push(piece.clone()),
      Role::Ink => path.extend(piece.iter()),
      // Shapes are visited in paint order, so what a paper shape means is
      // decided by what is already under it.
      Role::Paper => {
        if let Some(hole) = knocked_out_of(piece, &path) {
          path.extend(hole);
        }
      }
      Role::Wash => {}
    }
  }
  for body in &bodies {
    if let Some(cut) = region::subtract(body, &cuts) {
      path.extend(cut);
    }
  }
  path
}

/// Whether the ink already draws the outline this wash would.
///
/// A duotone icon is drawn twice: once faint for the body, once at full
/// strength for the outline over it. Where that second drawing is there, the
/// wash adds nothing a glyph can show; where it is not — a camera body, an
/// envelope — the wash is the only thing carrying the shape.
///
/// Judged on the outline the two enclose, since the ink is a stroke laid along
/// the wash's edge and so runs a stroke-width wider on every side.
fn traced_by(wash: &BezPath, ink: &[&BezPath]) -> bool {
  // A twentieth of the em, which is wider than any icon's stroke and narrower
  // than the gap between a body and a mark drawn inside it.
  const ALONG_THE_EDGE: f64 = UNITS_PER_EM as f64 / 20.0;

  let wanted = wash.bounding_box();
  ink.iter().any(|piece| {
    let edge = piece.bounding_box();
    (edge.x0 - wanted.x0).abs() < ALONG_THE_EDGE
      && (edge.y0 - wanted.y0).abs() < ALONG_THE_EDGE
      && (edge.x1 - wanted.x1).abs() < ALONG_THE_EDGE
      && (edge.y1 - wanted.y1).abs() < ALONG_THE_EDGE
  })
}

/// Whether `mark` is drawn inside `body` rather than merely crossing it.
///
/// Only a mark the body surrounds is one the body was drawn to carry. Two
/// shapes that lap over each other are two shapes, and both are ink.
fn encloses(body: &BezPath, mark: &BezPath) -> bool {
  body.bounding_box().contains_rect(mark.bounding_box())
}

/// One collected shape as an outline in font units: scaled, cut to the clips it
/// was drawn through, and wound so that it adds to its neighbours.
///
/// `None` when nothing is left, which a clip can genuinely do.
fn outline_of(filled: &Filled, scale: f64) -> Option<BezPath> {
  let mut piece = BezPath::new();
  append_scaled(&mut piece, &filled.path, scale);
  let mut piece = canonical_winding(&piece, filled.even_odd);

  for clip in &filled.clips {
    let mut region = BezPath::new();
    append_scaled(&mut region, clip, scale);
    let region = canonical_winding(&region, false);

    // A clip that surrounds what it clips is doing nothing, and that is what
    // almost every clip in the wild is: a drawing tool fencing off its
    // artboard. Recognising it keeps the resolver away from artwork that had
    // no question to ask, and keeps its curves exactly as they were drawn.
    if region::as_rectangle(&region).is_some_and(|rect| rect.contains_rect(piece.bounding_box())) {
      continue;
    }

    let cut = region::intersect(&piece, &region)?;
    // What a clip leaves cannot reach past either the shape or the clip. If it
    // does, the resolver has lost its way on some knot of tangencies, and the
    // shape as drawn is a far better answer than the clip's own outline —
    // which is what a failure here looks like.
    let within = |a: Rect, b: Rect| {
      const SLACK: f64 = 1.0;
      a.x0 >= b.x0 - SLACK && a.y0 >= b.y0 - SLACK && a.x1 <= b.x1 + SLACK && a.y1 <= b.y1 + SLACK
    };
    let bounds = cut.bounding_box();
    if within(bounds, piece.bounding_box()) && within(bounds, region.bounding_box()) {
      piece = cut;
    }
  }
  (!piece.is_empty()).then_some(piece)
}

/// Turn a shape's contours the same way round as every other shape's, without
/// changing what the shape itself fills.
///
/// A glyph is a single non-zero-filled path, so every shape an icon is built
/// from ends up in the same bag of contours. Non-zero filling adds windings
/// together, which means two overlapping contours that happen to turn opposite
/// ways *cancel*: a stroke laid over the fill it outlines eats a hole through
/// it, and an icon drawn as `fill` plus a matching `stroke` — a common way to
/// fatten artwork — comes out hollow.
///
/// Which way round a shape runs is not part of what the artwork meant, so the
/// fix is to impose one. The unit that can be turned is a contour *together
/// with everything nested inside it*: reversing a whole nest at once negates
/// its winding throughout, which leaves what it fills untouched — a hole stays
/// a hole, and an outline that crosses itself keeps whatever it drew. Turning
/// every nest so that its outermost contour runs positive leaves shapes that
/// can only add up.
///
/// Neither a smaller nor a larger unit will do, and both have been tried. Judge
/// each contour on its own and a stroked outline with a fold drawn into it — a
/// paper plane — loses the hole inside it, because the fold's ink sits over the
/// hole's edge and nothing local says which of the two it belonged to. Turn the
/// whole shape at once and a gear whose teeth and lettering are one `<path>`,
/// wound against each other, keeps the teeth backwards and they subtract from
/// the rim they sit on.
fn canonical_winding(path: &BezPath, even_odd: bool) -> BezPath {
  // Even-odd is the one case where contours really are re-wound against each
  // other: the rule itself has to be translated, since a glyph only knows
  // non-zero. What comes out is nested cleanly, which is what follows expects.
  let path = if even_odd {
    // Nesting decides which contours are holes, and that is enough whenever
    // the contours are simple closed loops. A contour that crosses itself has
    // no such answer — the middle of a five-pointed star is inside the one
    // contour twice — so there the region is resolved properly instead.
    if crosses_itself(path) {
      match region::even_odd_region(path) {
        Some(resolved) => resolved,
        // The resolver came back with nothing from a path that drew something,
        // so it could not make sense of the artwork. Nesting is a worse answer
        // than the truth but a better one than a blank glyph.
        None => region::even_odd_by_nesting(path),
      }
    } else {
      region::even_odd_by_nesting(path)
    }
  } else {
    path.clone()
  };

  let contours = split_contours(&path);
  if contours.len() < 2 {
    return if path.area() < 0.0 {
      path.reverse_subpaths()
    } else {
      path
    };
  }

  let mut out = BezPath::new();
  for (index, contour) in contours.iter().enumerate() {
    let nest = outermost_around(&contours, index).unwrap_or(index);
    if contours[nest].area() < 0.0 {
      out.extend(contour.reverse_subpaths().iter());
    } else {
      out.extend(contour.iter());
    }
  }
  out
}

/// The contour whose nest `index` sits in: the largest of those enclosing it,
/// or `None` when it enclosed by nothing and so is its own outermost.
///
/// Largest, rather than nearest, because it is the outermost contour that says
/// which way the whole nest runs. A contour is taken as enclosing when it holds
/// a point of the other — enough to tell nesting apart from lying alongside,
/// which is all this has to decide.
fn outermost_around(contours: &[BezPath], index: usize) -> Option<usize> {
  let probe = first_point(&contours[index])?;
  contours
    .iter()
    .enumerate()
    .filter(|(other, path)| *other != index && path.contains(probe))
    .max_by(|(_, a), (_, b)| {
      a.area()
        .abs()
        .partial_cmp(&b.area().abs())
        .unwrap_or(std::cmp::Ordering::Equal)
    })
    .map(|(other, _)| other)
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
    outline_with(svg, Color::Keep)
  }

  fn outline_with(svg: &str, color: Color) -> Outline {
    parse(svg.as_bytes(), "test.svg", color).unwrap()
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
  fn shapes_and_transforms_are_honored() {
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
    // The em center corresponds to the middle of the viewBox.
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

  const ONE_NAMED_COLOR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
             <path d="M2 2h20v20H2Z" fill="#3F3C43"/>
             <path d="M6 6h12v4H6Z" fill="#3F3C43"/>
           </svg>"##;

  #[test]
  fn a_single_named_color_is_kept() {
    // The artwork named a color, so by default it is a choice and stays. A
    // brand icon drawn in its brand color is the whole reason for the rule.
    let o = outline_with(ONE_NAMED_COLOR, Color::Keep);
    assert_eq!(o.coloring, Coloring::Fixed);
    assert_eq!(o.layers.len(), 1);
    assert_eq!(
      o.layers[0].paint,
      LayerPaint::Fixed {
        r: 0x3F,
        g: 0x3C,
        b: 0x43,
        a: 0xFF
      }
    );
  }

  #[test]
  fn recolor_single_reads_a_lone_color_as_the_foreground() {
    // Asked for, a lone color is read as a default rather than a choice, and
    // the icon follows the CSS `color` as if it had said `currentColor`.
    let o = outline_with(ONE_NAMED_COLOR, Color::RecolorSingle);
    assert_eq!(o.coloring, Coloring::Single);
    assert!(o.layers.is_empty());
  }

  #[test]
  fn recolor_drops_color_an_icon_uses_several_of() {
    let o = outline_with(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M2 2h20v20H2Z" fill="#FF4F00"/>
                  <path d="M6 6h12v4H6Z" fill="#0088CC"/>
                </svg>"##,
      Color::Recolor,
    );
    assert_eq!(o.coloring, Coloring::Single);
    assert!(o.layers.is_empty());
  }

  #[test]
  fn a_stroke_laid_over_a_fill_does_not_eat_it() {
    // A ring stroked over a panel of the same color — a badge with a circled
    // mark on it, and the shape of any number of brand icons. The ring's inner
    // contour used to wind against the panel and punch a hole clean through it.
    let panel_and_ring = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                 <path d="M2 2v20h20V2Z" fill="#000"/>
                 <path d="M12 6a6 6 0 1 0 0.01 0" fill="none" stroke="#000" stroke-width="4"/>
               </svg>"##;
    let panel_alone = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                 <path d="M2 2v20h20V2Z" fill="#000"/>
               </svg>"##;

    let with_ring = outline(panel_and_ring).path;
    let alone = outline(panel_alone).path;
    for x in 0..40 {
      for y in 0..40 {
        let at = Point::new(f64::from(x) * 25.0, f64::from(y) * 25.0 - 200.0);
        if alone.winding(at) != 0 {
          assert_ne!(with_ring.winding(at), 0, "the ring ate the panel at {at:?}");
        }
      }
    }
  }

  #[test]
  fn an_even_odd_star_that_crosses_itself_keeps_its_middle_empty() {
    // One contour, five crossings. Nesting has nothing to say about it — the
    // middle is inside the same contour twice — so the region is resolved.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <polygon fill-rule="evenodd" fill="#000"
                           points="12 2 4.6 21 21.4 8.9 2.6 8.9 19.4 21"/>
                </svg>"##,
    );
    assert_eq!(
      o.path.winding(Point::new(500.0, 380.0)),
      0,
      "the middle is a hole"
    );
    assert_ne!(
      o.path.winding(Point::new(500.0, 700.0)),
      0,
      "the top point is ink"
    );
  }

  #[test]
  fn a_clip_path_cuts_what_it_clips() {
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <defs><clipPath id="half"><rect x="0" y="0" width="24" height="12"/></clipPath></defs>
                  <circle cx="12" cy="12" r="9" fill="#000" clip-path="url(#half)"/>
                </svg>"##,
    );
    // The clip keeps the top half of the disc, which is the top of the em.
    assert_ne!(
      o.path.winding(Point::new(500.0, 500.0)),
      0,
      "the kept half is ink"
    );
    assert_eq!(
      o.path.winding(Point::new(500.0, 100.0)),
      0,
      "the cut half is gone"
    );
  }

  #[test]
  fn a_clip_that_surrounds_the_artwork_leaves_it_exactly_as_drawn() {
    // Every drawing tool fences its artboard off with one of these, so it has
    // to cost nothing — not even the re-cutting a resolver would do.
    let clipped = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <defs><clipPath id="all"><rect width="24" height="24"/></clipPath></defs>
                  <g clip-path="url(#all)"><circle cx="12" cy="12" r="9" fill="#000"/></g>
                </svg>"##,
    );
    let plain = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <circle cx="12" cy="12" r="9" fill="#000"/>
                </svg>"##,
    );
    assert_eq!(clipped.path, plain.path);
  }

  #[test]
  fn a_clip_is_read_in_the_space_of_whatever_refers_to_it() {
    // The clip covers the top half of its own coordinates, and the group flips
    // them, so what it really keeps is the bottom half. Read in the wrong space
    // it cuts the wrong half away — or, as here, nothing at all.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <defs><clipPath id="top"><rect width="24" height="12"/></clipPath></defs>
                  <g clip-path="url(#top)" transform="matrix(1 0 0 -1 0 24)">
                    <circle cx="12" cy="12" r="9" fill="#000"/>
                  </g>
                </svg>"##,
    );
    assert_eq!(
      o.path.winding(Point::new(500.0, 500.0)),
      0,
      "the flipped-away half is gone"
    );
    assert_ne!(
      o.path.winding(Point::new(500.0, 100.0)),
      0,
      "the kept half is ink"
    );
  }

  #[test]
  fn a_wash_the_ink_already_traces_is_dropped() {
    // The duotone battery: a faint body with its own outline stroked over it.
    // The outline says everything the body would, so the body adds nothing.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="4" y="4" width="16" height="16" fill="#000" opacity="0.2"/>
                  <rect x="4" y="4" width="16" height="16" fill="none"
                        stroke="#000" stroke-width="2"/>
                </svg>"##,
    );
    assert_eq!(
      o.path.winding(Point::new(500.0, 300.0)),
      0,
      "an outline, not a block"
    );
  }

  #[test]
  fn a_wash_the_ink_does_not_trace_becomes_the_body() {
    // The duotone camera: a faint body carrying the silhouette, with a mark
    // drawn inside it. Dropped, the icon is a lone mark and loses its subject;
    // painted over, the mark disappears into it. The mark becomes a hole.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="2" y="2" width="20" height="20" fill="#000" opacity="0.2"/>
                  <circle cx="12" cy="12" r="4" fill="none" stroke="#000" stroke-width="2"/>
                </svg>"##,
    );
    assert_ne!(
      o.path.winding(Point::new(200.0, 600.0)),
      0,
      "the body is drawn"
    );
    assert_eq!(
      o.path.winding(Point::new(500.0, 425.0)),
      0,
      "the mark is cut out of it"
    );
    assert_ne!(
      o.path.winding(Point::new(500.0, 300.0)),
      0,
      "and the middle of the mark is body"
    );
  }

  #[test]
  fn artwork_too_wide_for_a_glyph_is_reported_rather_than_chopped() {
    // Coordinates are 16-bit and the height is mapped onto the em, so past
    // about 32:1 the width has nowhere to go. Left alone, the rounding that
    // writes the glyph clamps it and the artwork is cut off partway across.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4000 100">
                  <rect width="4000" height="100" fill="#000"/>
                </svg>"##,
    );
    assert_eq!(o.problem, Some(Problem::TooWide));

    // And a merely wide icon is still perfectly buildable.
    let wide = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 100">
                  <rect width="240" height="100" fill="#000"/>
                </svg>"##,
    );
    assert_eq!(wide.problem, None);
  }

  #[test]
  fn a_line_given_svgs_default_fill_is_not_a_second_color() {
    // A path with only a `stroke` still arrives carrying SVG's default black
    // fill. A line has no inside, so it paints nothing — but counted as a color
    // it costs the icon the CSS `color`, and brings its faint layers up to full
    // strength. This is the duotone battery: a wash, an outline, and a terminal
    // drawn as a bare line.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="2" y="7" width="17" height="10" fill="currentColor" opacity="0.18"/>
                  <rect x="2" y="7" width="17" height="10" fill="none"
                        stroke="currentColor" stroke-width="1.8"/>
                  <path d="M21 10.5v3" stroke="currentColor" stroke-width="2.6"/>
                </svg>"##,
    );
    assert_eq!(o.coloring, Coloring::Single, "one color, and CSS sets it");
    // Every layer still follows the CSS `color`: the default fill did not
    // freeze one of them at black.
    assert!(o.layers.iter().all(|layer| layer.paint.follows_text()));
    // Nor did it bring the wash up to full strength.
    assert_eq!(
      o.layers.first().map(|layer| layer.paint),
      Some(LayerPaint::Foreground { a: 46 })
    );
    // The flattened outline, which is what a renderer without COLR falls back
    // to, still drops the wash rather than painting it as a solid block.
    assert_eq!(o.path.winding(Point::new(430.0, 300.0)), 0);
  }

  #[test]
  fn a_wash_in_the_text_color_is_kept_as_a_layer_of_its_own() {
    // A duotone icon drawn entirely in `currentColor` follows the CSS `color`
    // throughout, so it is not a color icon -- but a glyph has no opacity, and
    // the body at a fraction of full strength is half of what the icon is. It
    // is carried as COLR layers, which do have one.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="2" y="2" width="20" height="20" fill="currentColor" opacity="0.2"/>
                  <path d="M8 12.5 11 15.5 16.5 9" fill="none" stroke="currentColor"
                        stroke-width="2.2"/>
                </svg>"##,
    );
    assert_eq!(o.coloring, Coloring::Single);
    assert_eq!(
      o.layers.iter().map(|layer| layer.paint).collect::<Vec<_>>(),
      [
        LayerPaint::Foreground { a: 51 },
        LayerPaint::Foreground { a: 255 },
      ]
    );
  }

  #[test]
  fn one_strength_throughout_is_a_light_icon_rather_than_a_wash() {
    // Nothing is being held apart from anything, so there is no relationship
    // for a layer to carry: a glyph drawn at full strength is what this is.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="2" y="2" width="20" height="20" fill="currentColor" opacity="0.3"/>
                  <rect x="6" y="6" width="6" height="6" fill="currentColor" opacity="0.3"/>
                </svg>"##,
    );
    assert_eq!(o.coloring, Coloring::Single);
    assert!(o.layers.is_empty());
  }

  #[test]
  fn a_dashed_stroke_is_cut_into_its_dashes() {
    let dashed = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M2 12h20" fill="none" stroke="#000" stroke-width="2"
                        stroke-dasharray="4 4"/>
                </svg>"##,
    );
    let solid = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M2 12h20" fill="none" stroke="#000" stroke-width="2"/>
                </svg>"##,
    );
    // One contour per dash, against the solid rule's single one.
    assert!(
      dashed.path.segments().count() > solid.path.segments().count(),
      "a dashed rule should be several marks, not one"
    );
    // And it covers less ground, because the gaps are gaps.
    assert!(dashed.path.area().abs() < solid.path.area().abs() * 0.75);
  }

  #[test]
  fn a_line_drawn_into_a_stroked_outline_leaves_the_inside_empty() {
    // The telegram plane: one stroked outline with a fold drawn into it. The
    // fold's ink lies over the outline's inner edge, so nothing local says that
    // edge was the boundary of a hole — only the nest it belongs to does. Judge
    // the edge on its own and the plane fills in solid.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
                    stroke="#000" stroke-width="1.5" stroke-linejoin="round">
                  <path d="M4 4h16v16H4Z M12 12 4 4"/>
                </svg>"##,
    );
    // Well inside the square and clear of the diagonal, so only the outline
    // itself could have put ink here.
    assert_eq!(
      o.path.winding(Point::new(700.0, 200.0)),
      0,
      "the outline should enclose a hole, not a filled square"
    );
    // The outline itself is still drawn.
    assert_ne!(o.path.winding(Point::new(167.0, 633.0)), 0);
  }

  #[test]
  fn subpaths_wound_against_each_other_all_stay_ink() {
    // The cobol gear: teeth and lettering share one `<path>` and run opposite
    // ways. Each fills on its own, so both are ink — but the teeth sit on a rim
    // drawn by another path, and backwards they eat notches out of it.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path fill="#000" d="M4 4h16v16H4Z"/>
                  <path fill="#000" d="M0 10v4h6v-4Z M18 10h6v4h-6Z"/>
                </svg>"##,
    );
    // Where each tab laps onto the panel, both are ink and must stay ink.
    for x in [200.0, 800.0] {
      assert_ne!(
        o.path.winding(Point::new(x, 300.0)),
        0,
        "a tab lapping the panel at x = {x} should not cut into it"
      );
    }
  }

  #[test]
  fn overlapping_shapes_add_up_instead_of_cancelling() {
    // Two squares drawn the opposite way round the same way: whatever the
    // artwork wound them, the overlap is ink in both, so it stays ink.
    let o = outline(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <path d="M2 2h12v12H2Z" fill="#000"/>
                  <path d="M10 10V22h12V10Z" fill="#000"/>
                </svg>"##,
    );
    // A point in the overlap of the two squares.
    assert_ne!(o.path.winding(Point::new(500.0, 300.0)), 0);
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
  fn two_colors_become_layers_in_paint_order() {
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
  fn current_color_survives_beside_a_fixed_color() {
    // The tick follows the CSS color while the disc stays blue.
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
        LayerPaint::Foreground { a: 255 },
      ]
    );
  }

  #[test]
  fn an_svg_that_sets_its_own_color_keeps_it() {
    // Here `currentColor` names a color the artwork chose, so it is not
    // the foreground and the icon is a two-color one.
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
  fn the_flattened_outline_is_still_built_for_a_color_icon() {
    // COLR needs a base glyph for renderers without color support.
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
