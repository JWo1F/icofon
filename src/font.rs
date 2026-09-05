//! Assembling the parsed icons into a TrueType font.

use anyhow::{Context, Result, bail};
use write_fonts::{
  FontBuilder,
  tables::{
    cmap::Cmap,
    colr::{
      BaseGlyph, BaseGlyphList, BaseGlyphPaint, Colr, Layer as ColrLayer, LayerList, Paint,
      PaintColrLayers, PaintGlyph, PaintSolid,
    },
    cpal::{ColorRecord, Cpal},
    glyf::{Bbox, GlyfLocaBuilder, Glyph, SimpleGlyph},
    head::{Flags, Head},
    hhea::Hhea,
    hmtx::{Hmtx, LongMetric},
    maxp::Maxp,
    name::{Name, NameRecord},
    os2::{Os2, SelectionFlags},
    post::Post,
  },
  types::{
    F2Dot14, FWord, Fixed, GlyphId, GlyphId16, LongDateTime, NameId, Tag, UfWord, Version16Dot16,
  },
};

use crate::svg::{LayerPaint, Outline};

/// Design grid. 1000 units per em keeps the numbers readable and is what most
/// icon tooling expects.
pub const UNITS_PER_EM: u16 = 1000;
/// Top of the em box. Together with `DESCENDER` this spans exactly one em, so
/// an icon drawn edge to edge in its viewBox renders one em tall and sits
/// slightly below the baseline — the position that lines up with running text.
pub const ASCENDER: i16 = 800;
/// Bottom of the em box.
pub const DESCENDER: i16 = -200;

/// One layer's strength as COLR writes it: 0 is invisible, 1 is the color at
/// full.
fn opacity(alpha: u8) -> F2Dot14 {
  F2Dot14::from_f32(f32::from(alpha) / 255.0)
}

/// Seconds between the TrueType epoch (1904-01-01) and the Unix epoch.
const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;

/// Creation date stamped into every font, as a Unix timestamp: 2024-01-01.
///
/// Deliberately fixed rather than "now". The same icons must compile to the
/// same bytes every time, or a font committed alongside its sources shows up as
/// changed on every build and every rebuild is a diff to review.
const BUILD_TIMESTAMP: i64 = 1_704_067_200;

/// Advance given to `.notdef`, which we emit as a blank glyph.
const NOTDEF_ADVANCE: u16 = UNITS_PER_EM / 2;

#[derive(Debug)]
pub struct Icon {
  /// Unique name used for the CSS class and the manifest, built from the
  /// subfolders and the file name: `arrows/left.svg` is `arrows-left`.
  pub name: String,
  /// Just the file part of the name. The preview page shows this under the
  /// group heading, so a card is not made of the folder repeated three times.
  pub label: String,
  /// Subfolder the icon came from, used to group the preview page.
  pub group: Option<String>,
  /// Where the icon was read from, so problems can name the actual file.
  pub source: std::path::PathBuf,
  pub codepoint: char,
  pub outline: Outline,
}

/// Compile `icons` into the bytes of a TrueType font.
pub fn build(icons: &[Icon], family: &str) -> Result<Vec<u8>> {
  let mut glyf = GlyfLocaBuilder::new();
  let mut metrics = vec![LongMetric::new(NOTDEF_ADVANCE, 0)];
  let mut mappings = Vec::with_capacity(icons.len());
  let mut bounds: Option<Bounds> = None;
  let (mut max_points, mut max_contours) = (0u16, 0u16);

  glyf
    .add_glyph(&Glyph::Empty)
    .context("adding the .notdef glyph")?;

  let mut color = ColorTables::default();
  // Layer glyphs are appended after every base glyph, so their ids start past
  // .notdef and the icons.
  let mut next_layer_gid = 1 + icons.len();
  let mut layer_glyphs = Vec::new();

  for (index, icon) in icons.iter().enumerate() {
    let glyph = match SimpleGlyph::from_bezpath(&icon.outline.path) {
      Ok(glyph) => {
        let points: usize = glyph.contours.iter().map(|c| c.iter().count()).sum();
        max_points = max_points.max(points.try_into().unwrap_or(u16::MAX));
        max_contours = max_contours.max(glyph.contours.len().try_into().unwrap_or(u16::MAX));
        Glyph::Simple(glyph)
      }
      // An icon whose SVG contains nothing drawable is still worth a
      // glyph and a codepoint; it just renders as blank.
      Err(_) => Glyph::Empty,
    };

    let bbox = glyph.bbox();
    let side_bearing = bbox.map_or(0, |b| b.x_min);
    if let Some(b) = bbox {
      bounds = Some(match bounds {
        Some(existing) => existing.union(b, icon.outline.advance),
        None => Bounds::new(b, icon.outline.advance),
      });
    }

    glyf
      .add_glyph(&glyph)
      .with_context(|| format!("compiling glyph for icon '{}'", icon.name))?;
    metrics.push(LongMetric::new(icon.outline.advance, side_bearing));
    // Glyph 0 is .notdef, so the first icon lands at glyph 1.
    let base_gid = index + 1;
    mappings.push((icon.codepoint, GlyphId::new(base_gid as u32)));

    // A color icon also gets one glyph per layer, referenced from COLR.
    // The base glyph above stays as the flattened outline, which is what a
    // renderer without color support falls back to.
    let mut layers = Vec::new();
    for layer in &icon.outline.layers {
      // An icon painted in one color is its own only layer, so the layer would
      // be a byte-for-byte copy of the base glyph. Point at the base instead:
      // a single-color set is otherwise carried twice over.
      if layer.path == icon.outline.path {
        layers.push((base_gid, layer.paint));
        continue;
      }
      let Ok(glyph) = SimpleGlyph::from_bezpath(&layer.path) else {
        continue;
      };
      let points: usize = glyph.contours.iter().map(|c| c.iter().count()).sum();
      max_points = max_points.max(points.try_into().unwrap_or(u16::MAX));
      max_contours = max_contours.max(glyph.contours.len().try_into().unwrap_or(u16::MAX));
      if let Some(b) = Some(glyph.bbox) {
        bounds = Some(match bounds {
          Some(existing) => existing.union(b, icon.outline.advance),
          None => Bounds::new(b, icon.outline.advance),
        });
      }

      layers.push((next_layer_gid, layer.paint));
      layer_glyphs.push((glyph, icon.outline.advance));
      next_layer_gid += 1;
    }
    color.add(base_gid, &layers, &icon.name)?;
  }

  for (glyph, advance) in layer_glyphs {
    let side_bearing = glyph.bbox.x_min;
    glyf.add_glyph(&glyph).context("compiling a color layer")?;
    metrics.push(LongMetric::new(advance, side_bearing));
  }

  let (glyf, loca, loca_format) = glyf.build();
  let bounds = bounds.unwrap_or_default();
  // A font addresses its glyphs with a u16, so the count is a hard ceiling
  // rather than something to truncate past: a colored icon costs a glyph per
  // layer as well as its own, so a large set reaches this sooner than its icon
  // count suggests.
  let num_glyphs = u16::try_from(metrics.len()).map_err(|_| {
    anyhow::anyhow!(
      "{} glyphs is more than a font can address; a font holds at most {} \
             (colored icons cost one glyph per color as well as their own)",
      metrics.len(),
      u16::MAX
    )
  })?;
  let max_advance = metrics.iter().map(|m| m.advance).max().unwrap_or(0);
  let average_advance =
    (metrics.iter().map(|m| u32::from(m.advance)).sum::<u32>() / u32::from(num_glyphs)) as i16;

  let head = Head {
    font_revision: Fixed::from_f64(1.0),
    // Baseline at y=0 and left side bearing at x=0, both true by construction.
    flags: Flags::from_bits_truncate(0b11),
    units_per_em: UNITS_PER_EM,
    created: stamp(),
    modified: stamp(),
    x_min: bounds.x_min,
    y_min: bounds.y_min,
    x_max: bounds.x_max,
    y_max: bounds.y_max,
    lowest_rec_ppem: 8,
    index_to_loc_format: loca_format as i16,
    ..Default::default()
  };

  let hhea = Hhea {
    ascender: FWord::new(ASCENDER),
    descender: FWord::new(DESCENDER),
    line_gap: FWord::new(0),
    advance_width_max: UfWord::new(max_advance),
    min_left_side_bearing: FWord::new(bounds.min_lsb),
    min_right_side_bearing: FWord::new(bounds.min_rsb),
    x_max_extent: FWord::new(bounds.x_max),
    caret_slope_rise: 1,
    number_of_h_metrics: num_glyphs,
    ..Default::default()
  };

  let maxp = Maxp {
    num_glyphs,
    max_points: Some(max_points),
    max_contours: Some(max_contours),
    max_composite_points: Some(0),
    max_composite_contours: Some(0),
    max_zones: Some(2),
    max_twilight_points: Some(0),
    max_storage: Some(0),
    max_function_defs: Some(0),
    max_instruction_defs: Some(0),
    max_stack_elements: Some(0),
    max_size_of_instructions: Some(0),
    max_component_elements: Some(0),
    max_component_depth: Some(0),
  };

  let os2 = Os2 {
    x_avg_char_width: average_advance,
    us_weight_class: 400,
    us_width_class: 5,
    y_strikeout_size: 50,
    y_strikeout_position: 300,
    ul_unicode_range_2: unicode_range_2(icons),
    ach_vend_id: Tag::new(b"NONE"),
    fs_selection: SelectionFlags::REGULAR,
    us_first_char_index: icons
      .iter()
      .map(|i| i.codepoint as u32)
      .min()
      .unwrap_or(0)
      .min(u32::from(u16::MAX)) as u16,
    us_last_char_index: icons
      .iter()
      .map(|i| i.codepoint as u32)
      .max()
      .unwrap_or(0)
      .min(u32::from(u16::MAX)) as u16,
    s_typo_ascender: ASCENDER,
    s_typo_descender: DESCENDER,
    s_typo_line_gap: 0,
    us_win_ascent: ASCENDER as u16,
    us_win_descent: DESCENDER.unsigned_abs(),
    ul_code_page_range_1: Some(1),
    ul_code_page_range_2: Some(0),
    sx_height: Some(0),
    s_cap_height: Some(ASCENDER),
    us_default_char: Some(0),
    us_break_char: Some(32),
    us_max_context: Some(0),
    ..Default::default()
  };

  let post = Post {
    version: Version16Dot16::VERSION_3_0,
    underline_position: FWord::new(-100),
    underline_thickness: FWord::new(50),
    ..Default::default()
  };

  let cmap = Cmap::from_mappings(mappings).context("building the character map")?;
  let hmtx = Hmtx::new(metrics, Vec::new());

  let mut builder = FontBuilder::new();
  builder.add_table(&head)?;
  builder.add_table(&hhea)?;
  builder.add_table(&maxp)?;
  builder.add_table(&os2)?;
  builder.add_table(&hmtx)?;
  builder.add_table(&cmap)?;
  builder.add_table(&loca)?;
  builder.add_table(&glyf)?;
  builder.add_table(&name_table(family))?;
  builder.add_table(&post)?;
  if let Some((colr, cpal)) = color.build() {
    builder.add_table(&colr)?;
    builder.add_table(&cpal)?;
  }
  Ok(builder.build())
}

/// Accumulates the COLR and CPAL tables as color icons are compiled.
///
/// COLR maps a base glyph to a run of layers, each naming a glyph and a color
/// from the CPAL palette. The reserved palette index 0xFFFF means "the text
/// color", which is how a `currentColor` layer keeps following CSS `color`
/// while its neighbors stay the color the artwork chose.
#[derive(Default)]
struct ColorTables {
  base_glyphs: Vec<BaseGlyph>,
  layers: Vec<ColrLayer>,
  /// The COLR v1 side of the table: one paint graph per icon that needs a
  /// layer painted at less than full strength. Only those icons go here — a
  /// v0 record says everything the others need, and is understood by more.
  base_paints: Vec<BaseGlyphPaint>,
  paints: Vec<Paint>,
  /// Distinct fixed colors, in the order first seen; the index into this is
  /// the CPAL palette index.
  palette: Vec<(u8, u8, u8, u8)>,
}

/// The palette index OpenType reserves for the current text color.
const FOREGROUND_PALETTE_INDEX: u16 = 0xFFFF;

impl ColorTables {
  fn add(&mut self, base_gid: usize, layers: &[(usize, LayerPaint)], name: &str) -> Result<()> {
    if layers.is_empty() {
      return Ok(());
    }
    // A COLR v0 layer is a glyph and a palette index and nothing more, and the
    // index reserved for the text color has no alpha to give. So an icon that
    // draws the text color at less than full strength — a duotone body under
    // its own outline — can only be said in COLR v1, where a paint carries its
    // own alpha. Everything else stays v0.
    if layers
      .iter()
      .any(|(_, paint)| matches!(paint, LayerPaint::Foreground { a } if *a != u8::MAX))
    {
      return self.add_translucent(base_gid, layers, name);
    }

    let first = u16::try_from(self.layers.len())
      .with_context(|| format!("too many color layers by the time '{name}' was reached"))?;
    let count =
      u16::try_from(layers.len()).with_context(|| format!("'{name}' has too many color layers"))?;

    for (gid, paint) in layers {
      let palette_index = match paint {
        LayerPaint::Foreground { .. } => FOREGROUND_PALETTE_INDEX,
        LayerPaint::Fixed { r, g, b, a } => self.color_index((*r, *g, *b, *a), name)?,
      };
      self
        .layers
        .push(ColrLayer::new(small_gid(*gid, name)?, palette_index));
    }

    self
      .base_glyphs
      .push(BaseGlyph::new(small_gid(base_gid, name)?, first, count));
    Ok(())
  }

  /// The same icon as a COLR v1 paint graph: one `PaintGlyph` per layer, each
  /// filled with a `PaintSolid` that carries the layer's own alpha.
  fn add_translucent(
    &mut self,
    base_gid: usize,
    layers: &[(usize, LayerPaint)],
    name: &str,
  ) -> Result<()> {
    let first = u32::try_from(self.paints.len())
      .with_context(|| format!("too many color layers by the time '{name}' was reached"))?;
    // A v1 paint graph counts its layers in a single byte.
    let count = u8::try_from(layers.len()).with_context(|| {
      format!("'{name}' is drawn in more than 255 runs of color, which COLR cannot hold")
    })?;

    for (gid, paint) in layers {
      let solid = match paint {
        LayerPaint::Foreground { a } => PaintSolid::new(FOREGROUND_PALETTE_INDEX, opacity(*a)),
        // A palette entry carries its own alpha, so the paint asks for all of
        // what the entry already is.
        LayerPaint::Fixed { r, g, b, a } => PaintSolid::new(
          self.color_index((*r, *g, *b, *a), name)?,
          F2Dot14::from_f32(1.0),
        ),
      };
      self.paints.push(Paint::Glyph(PaintGlyph::new(
        Paint::Solid(solid),
        small_gid(*gid, name)?,
      )));
    }

    self.base_paints.push(BaseGlyphPaint::new(
      small_gid(base_gid, name)?,
      Paint::ColrLayers(PaintColrLayers::new(count, first)),
    ));
    Ok(())
  }

  fn color_index(&mut self, color: (u8, u8, u8, u8), name: &str) -> Result<u16> {
    if let Some(at) = self.palette.iter().position(|entry| *entry == color) {
      return Ok(at as u16);
    }
    let at = u16::try_from(self.palette.len())
      .with_context(|| format!("more than 65535 colors by the time '{name}' was reached"))?;
    // 0xFFFF is reserved for the text color, so it cannot also be a slot.
    if at == FOREGROUND_PALETTE_INDEX {
      bail!("the palette is full");
    }
    self.palette.push(color);
    Ok(at)
  }

  fn build(self) -> Option<(Colr, Cpal)> {
    if self.base_glyphs.is_empty() && self.base_paints.is_empty() {
      return None;
    }

    let entries = self.palette.len().max(1) as u16;
    let records = if self.palette.is_empty() {
      // A palette must have at least one entry even when every layer
      // follows the text color.
      vec![ColorRecord::new(0, 0, 0, 255)]
    } else {
      self
        .palette
        .iter()
        .map(|(r, g, b, a)| ColorRecord::new(*b, *g, *r, *a))
        .collect()
    };

    let mut colr = Colr::new(
      self.base_glyphs.len() as u16,
      Some(self.base_glyphs),
      Some(self.layers.clone()),
      self.layers.len() as u16,
    );
    // Written only when something needs it: the version the table declares
    // follows from whether these are set, and a font of plain color icons
    // stays a v0 font that every COLR renderer understands.
    if !self.base_paints.is_empty() {
      colr.base_glyph_list = Some(BaseGlyphList::new(
        self.base_paints.len() as u32,
        self.base_paints,
      ))
      .into();
      colr.layer_list = Some(LayerList::new(self.paints.len() as u32, self.paints)).into();
    }
    let cpal = Cpal::new(entries, 1, entries, Some(records), vec![0]);
    Some((colr, cpal))
  }
}

fn small_gid(gid: usize, name: &str) -> Result<GlyphId16> {
  u16::try_from(gid)
    .map(GlyphId16::new)
    .with_context(|| format!("'{name}' pushed the font past 65535 glyphs"))
}

/// Running tally of the font-wide extents that head and hhea need.
#[derive(Clone, Copy, Default)]
struct Bounds {
  x_min: i16,
  y_min: i16,
  x_max: i16,
  y_max: i16,
  min_lsb: i16,
  min_rsb: i16,
}

impl Bounds {
  fn new(bbox: Bbox, advance: u16) -> Self {
    Self {
      x_min: bbox.x_min,
      y_min: bbox.y_min,
      x_max: bbox.x_max,
      y_max: bbox.y_max,
      min_lsb: bbox.x_min,
      min_rsb: (i32::from(advance) - i32::from(bbox.x_max))
        .clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
    }
  }

  fn union(self, bbox: Bbox, advance: u16) -> Self {
    let other = Self::new(bbox, advance);
    Self {
      x_min: self.x_min.min(other.x_min),
      y_min: self.y_min.min(other.y_min),
      x_max: self.x_max.max(other.x_max),
      y_max: self.y_max.max(other.y_max),
      min_lsb: self.min_lsb.min(other.min_lsb),
      min_rsb: self.min_rsb.min(other.min_rsb),
    }
  }
}

/// Bit 60 of the OS/2 unicode ranges covers the Private Use Area, which is
/// where icon codepoints normally live.
fn unicode_range_2(icons: &[Icon]) -> u32 {
  let uses_pua = icons
    .iter()
    .any(|i| ('\u{e000}'..='\u{f8ff}').contains(&i.codepoint));
  if uses_pua { 1 << 28 } else { 0 }
}

fn name_table(family: &str) -> Name {
  let postscript = family.replace(' ', "");
  let entries = [
    (NameId::FAMILY_NAME, family.to_string()),
    (NameId::SUBFAMILY_NAME, "Regular".to_string()),
    (NameId::UNIQUE_ID, format!("{family} Regular")),
    (NameId::FULL_NAME, format!("{family} Regular")),
    (NameId::VERSION_STRING, "Version 1.0".to_string()),
    (NameId::POSTSCRIPT_NAME, postscript),
  ];

  let mut records: Vec<_> = entries
    .into_iter()
    .flat_map(|(id, value)| {
      [
        // Unicode platform, and Windows platform with US English.
        NameRecord::new(0, 3, 0, id, value.clone().into()),
        NameRecord::new(3, 1, 0x409, id, value.into()),
      ]
    })
    .collect();
  // The name table requires its records in platform/encoding/language/name order.
  records.sort_by_key(|r| (r.platform_id, r.encoding_id, r.language_id, r.name_id));
  Name::new(records)
}

fn stamp() -> LongDateTime {
  LongDateTime::new(BUILD_TIMESTAMP + MAC_EPOCH_OFFSET)
}

#[cfg(test)]
mod tests {
  use super::*;
  use read_fonts::{FontRef, TableProvider};

  fn icon(name: &str, codepoint: char, svg: &str) -> Icon {
    Icon {
      name: name.to_string(),
      label: name.to_string(),
      group: None,
      source: name.into(),
      codepoint,
      outline: crate::svg::parse(svg.as_bytes(), name, crate::config::Color::Keep).unwrap(),
    }
  }

  fn square(size: &str) -> String {
    format!(
      r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="4" y="4" width="{size}" height="{size}" fill="currentColor"/>
                </svg>"##
    )
  }

  #[test]
  fn produces_a_readable_font() {
    let icons = vec![
      icon("small", '\u{e900}', &square("8")),
      icon("large", '\u{e901}', &square("16")),
    ];
    let bytes = build(&icons, "Test Icons").unwrap();
    let font = FontRef::new(&bytes).expect("the output should parse as a font");

    let head = font.head().unwrap();
    assert_eq!(head.units_per_em(), UNITS_PER_EM);

    // .notdef plus one glyph per icon.
    assert_eq!(font.maxp().unwrap().num_glyphs(), 3);

    let cmap = font.cmap().unwrap();
    assert_eq!(cmap.map_codepoint('\u{e900}').unwrap().to_u32(), 1);
    assert_eq!(cmap.map_codepoint('\u{e901}').unwrap().to_u32(), 2);
    assert!(cmap.map_codepoint('a').is_none());

    // Both glyphs carry real outlines.
    let glyf = font.glyf().unwrap();
    let loca = font.loca(None).unwrap();
    for gid in [1u32, 2] {
      let glyph = loca
        .get_glyf(GlyphId::new(gid), &glyf)
        .unwrap()
        .expect("icon glyphs should not be empty");
      assert!(glyph.number_of_contours() > 0);
    }
  }

  #[test]
  fn a_color_icon_gets_colr_and_cpal() {
    let colorful = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                              <path d="M0 0h24v24H0Z" fill="#FF4F00"/>
                              <path d="M6 6h12v12H6Z" fill="#FFFFFF"/>
                            </svg>"##;
    let icons = vec![
      icon("plain", '\u{e900}', &square("8")),
      icon("brand", '\u{e901}', colorful),
    ];
    let bytes = build(&icons, "Test Icons").unwrap();
    let font = FontRef::new(&bytes).unwrap();

    // .notdef, two base glyphs, and two layer glyphs for the color icon.
    assert_eq!(font.maxp().unwrap().num_glyphs(), 5);

    let colr = font.colr().expect("a color icon should produce COLR");
    let records = colr.base_glyph_records().unwrap().unwrap();
    assert_eq!(records.len(), 1, "only the color icon gets a record");
    assert_eq!(records[0].glyph_id().to_u32(), 2);
    assert_eq!(records[0].num_layers(), 2);

    let cpal = font.cpal().expect("color layers need a palette");
    assert_eq!(cpal.num_palette_entries(), 2);
  }

  #[test]
  fn a_wash_in_the_text_color_is_written_as_a_colr_v1_paint() {
    // A duotone icon drawn wholly in `currentColor`: a body at a fraction of
    // full strength under an outline at full. COLR v0 layers cannot say that --
    // the palette entry reserved for the text color has no alpha -- so it goes
    // in the v1 list, where a paint carries its own.
    let duotone = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                             <rect x="2" y="2" width="20" height="20" fill="currentColor"
                                   opacity="0.2"/>
                             <rect x="2" y="2" width="20" height="20" fill="none"
                                   stroke="currentColor" stroke-width="2"/>
                           </svg>"##;
    let bytes = build(&[icon("battery", '\u{e900}', duotone)], "Test Icons").unwrap();
    let font = FontRef::new(&bytes).unwrap();

    let colr = font.colr().expect("a wash needs COLR to carry it");
    assert_eq!(colr.version(), 1);
    // Nothing is left in the v0 arrays, so a renderer that only knows v0 falls
    // back to the flattened outline rather than painting the wash solid.
    assert_eq!(colr.num_base_glyph_records(), 0);

    let list = colr.base_glyph_list().unwrap().unwrap();
    let records = list.base_glyph_paint_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].glyph_id().to_u32(), 1);

    let layers = colr.layer_list().unwrap().unwrap();
    assert_eq!(layers.num_layers(), 2);

    // The bottom layer is the text color at a fifth of full strength.
    let read_fonts::tables::colr::Paint::Glyph(glyph) = layers.paints().get(0).unwrap() else {
      panic!("a layer is a glyph filled with a paint");
    };
    let read_fonts::tables::colr::Paint::Solid(solid) = glyph.paint().unwrap() else {
      panic!("the fill is one flat color");
    };
    assert_eq!(solid.palette_index(), FOREGROUND_PALETTE_INDEX);
    assert!(
      (solid.alpha().to_f32() - 0.2).abs() < 0.01,
      "{}",
      solid.alpha().to_f32()
    );
  }

  #[test]
  fn a_font_of_plain_icons_has_no_color_tables() {
    let icons = vec![icon("plain", '\u{e900}', &square("8"))];
    let bytes = build(&icons, "Test Icons").unwrap();
    let font = FontRef::new(&bytes).unwrap();
    assert!(font.colr().is_err(), "no COLR without a color icon");
    assert!(font.cpal().is_err());
  }

  #[test]
  fn vertical_metrics_span_exactly_one_em() {
    let icons = vec![icon("only", '\u{e900}', &square("16"))];
    let bytes = build(&icons, "Test Icons").unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let hhea = font.hhea().unwrap();
    assert_eq!(
      i32::from(hhea.ascender().to_i16()) - i32::from(hhea.descender().to_i16()),
      i32::from(UNITS_PER_EM)
    );
  }
}
