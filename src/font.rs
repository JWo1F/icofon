//! Assembling the parsed icons into a TrueType font.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use write_fonts::{
    FontBuilder,
    tables::{
        cmap::Cmap,
        glyf::{Bbox, GlyfLocaBuilder, Glyph, SimpleGlyph},
        head::{Flags, Head},
        hhea::Hhea,
        hmtx::{Hmtx, LongMetric},
        maxp::Maxp,
        name::{Name, NameRecord},
        os2::{Os2, SelectionFlags},
        post::Post,
    },
    types::{FWord, Fixed, GlyphId, LongDateTime, NameId, Tag, UfWord, Version16Dot16},
};

use crate::svg::Outline;

/// Design grid. 1000 units per em keeps the numbers readable and is what most
/// icon tooling expects.
pub const UNITS_PER_EM: u16 = 1000;
/// Top of the em box. Together with `DESCENDER` this spans exactly one em, so
/// an icon drawn edge to edge in its viewBox renders one em tall and sits
/// slightly below the baseline — the position that lines up with running text.
pub const ASCENDER: i16 = 800;
/// Bottom of the em box.
pub const DESCENDER: i16 = -200;

/// Seconds between the TrueType epoch (1904-01-01) and the Unix epoch.
const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;

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

    glyf.add_glyph(&Glyph::Empty)
        .context("adding the .notdef glyph")?;

    for (index, icon) in icons.iter().enumerate() {
        let glyph = match SimpleGlyph::from_bezpath(&icon.outline.path) {
            Ok(glyph) => {
                let points: usize = glyph.contours.iter().map(|c| c.iter().count()).sum();
                max_points = max_points.max(points.try_into().unwrap_or(u16::MAX));
                max_contours =
                    max_contours.max(glyph.contours.len().try_into().unwrap_or(u16::MAX));
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

        glyf.add_glyph(&glyph)
            .with_context(|| format!("compiling glyph for icon '{}'", icon.name))?;
        metrics.push(LongMetric::new(icon.outline.advance, side_bearing));
        // Glyph 0 is .notdef, so the first icon lands at glyph 1.
        mappings.push((icon.codepoint, GlyphId::new((index + 1) as u32)));
    }

    let (glyf, loca, loca_format) = glyf.build();
    let bounds = bounds.unwrap_or_default();
    let num_glyphs = metrics.len() as u16;
    let max_advance = metrics.iter().map(|m| m.advance).max().unwrap_or(0);
    let average_advance =
        (metrics.iter().map(|m| u32::from(m.advance)).sum::<u32>() / u32::from(num_glyphs)) as i16;

    let head = Head {
        font_revision: Fixed::from_f64(1.0),
        // Baseline at y=0 and left side bearing at x=0, both true by construction.
        flags: Flags::from_bits_truncate(0b11),
        units_per_em: UNITS_PER_EM,
        created: now(),
        modified: now(),
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
        us_first_char_index: icons.iter().map(|i| i.codepoint as u32).min().unwrap_or(0) as u16,
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
    Ok(builder.build())
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

fn now() -> LongDateTime {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    LongDateTime::new(unix + MAC_EPOCH_OFFSET)
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
            codepoint,
            outline: crate::svg::parse(svg.as_bytes(), name).unwrap(),
        }
    }

    fn square(size: &str) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                  <rect x="4" y="4" width="{size}" height="{size}" fill="#000"/>
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
