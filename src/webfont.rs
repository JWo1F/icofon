//! Web font containers: WOFF and WOFF2.
//!
//! Both are wrappers around the same TrueType tables, not different outlines.
//! WOFF deflates each table on its own; WOFF2 brotli-compresses the whole run
//! of them at once, which is why it wins so clearly.
//!
//! The glyf/loca transform WOFF2 also allows is deliberately not implemented.
//! It re-encodes the outlines into a bespoke representation to save a further
//! few percent, and getting it wrong corrupts glyphs rather than failing. The
//! spec provides a null transform for exactly this case, so the tables are
//! stored as they are and brotli does the work.

use std::io::Write as _;

use anyhow::{Context, Result, bail};

const WOFF: u32 = 0x774F_4646; // 'wOFF'
const WOFF2: u32 = 0x774F_4632; // 'wOF2'

/// One table lifted out of the sfnt the font builder produced.
struct Table {
  tag: [u8; 4],
  checksum: u32,
  data: Vec<u8>,
}

/// Read the table directory of a TrueType font.
fn tables(sfnt: &[u8]) -> Result<(u32, Vec<Table>)> {
  let read_u32 = |at: usize| -> Result<u32> {
    let bytes = sfnt
      .get(at..at + 4)
      .context("font is truncated inside its table directory")?;
    Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
  };

  let flavor = read_u32(0)?;
  let count = u16::from_be_bytes(
    sfnt
      .get(4..6)
      .context("font is too short to have a table directory")?
      .try_into()
      .unwrap(),
  ) as usize;

  let mut tables = Vec::with_capacity(count);
  for i in 0..count {
    let entry = 12 + i * 16;
    let tag: [u8; 4] = sfnt
      .get(entry..entry + 4)
      .context("font is truncated inside its table directory")?
      .try_into()
      .unwrap();
    let checksum = read_u32(entry + 4)?;
    let offset = read_u32(entry + 8)? as usize;
    let length = read_u32(entry + 12)? as usize;
    let data = sfnt
      .get(offset..offset + length)
      .with_context(|| {
        format!(
          "table {} runs past the end of the font",
          String::from_utf8_lossy(&tag)
        )
      })?
      .to_vec();
    tables.push(Table {
      tag,
      checksum,
      data,
    });
  }

  // Both containers want the directory in tag order. An sfnt is already
  // written that way, but nothing here depends on the builder keeping to it.
  tables.sort_by_key(|table| table.tag);
  Ok((flavor, tables))
}

/// The size the font takes up once a decoder has rebuilt it as an sfnt, which
/// both containers have to declare up front.
fn sfnt_size(tables: &[Table]) -> u32 {
  let directory = 12 + 16 * tables.len();
  let payload: usize = tables.iter().map(|t| pad4(t.data.len())).sum();
  (directory + payload) as u32
}

fn pad4(n: usize) -> usize {
  n.div_ceil(4) * 4
}

/// Wrap a TrueType font as WOFF, deflating each table separately.
pub fn woff(sfnt: &[u8]) -> Result<Vec<u8>> {
  let (flavor, tables) = tables(sfnt)?;

  // Compress first: a table only goes in compressed if that actually made it
  // smaller, which the reader detects by compLength == origLength.
  let mut bodies = Vec::with_capacity(tables.len());
  for table in &tables {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&table.data)?;
    let deflated = encoder.finish()?;
    bodies.push(if deflated.len() < table.data.len() {
      deflated
    } else {
      table.data.clone()
    });
  }

  let header = 44;
  let directory = 20 * tables.len();
  let mut offset = header + directory;

  let mut out = Vec::new();
  out.extend_from_slice(&WOFF.to_be_bytes());
  out.extend_from_slice(&flavor.to_be_bytes());
  let total: usize = offset + bodies.iter().map(|b| pad4(b.len())).sum::<usize>();
  out.extend_from_slice(&(total as u32).to_be_bytes());
  out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
  out.extend_from_slice(&0u16.to_be_bytes()); // reserved
  out.extend_from_slice(&sfnt_size(&tables).to_be_bytes());
  out.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
  out.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
  for _ in 0..5 {
    out.extend_from_slice(&0u32.to_be_bytes()); // no metadata, no private block
  }

  for (table, body) in tables.iter().zip(&bodies) {
    out.extend_from_slice(&table.tag);
    out.extend_from_slice(&(offset as u32).to_be_bytes());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&(table.data.len() as u32).to_be_bytes());
    out.extend_from_slice(&table.checksum.to_be_bytes());
    offset += pad4(body.len());
  }

  for body in &bodies {
    out.extend_from_slice(body);
    out.resize(pad4(out.len()), 0);
  }

  Ok(out)
}

/// Wrap a TrueType font as WOFF2: one brotli stream over every table.
pub fn woff2(sfnt: &[u8]) -> Result<Vec<u8>> {
  let (flavor, tables) = tables(sfnt)?;

  let mut directory = Vec::new();
  for table in &tables {
    // Bits 0-5 name the table. 63 means "the tag follows", which every table
    // may use; spelling the tag out costs four bytes and avoids depending on
    // the known-table list. Bits 6-7 are the transform version, and for glyf
    // and loca 3 is the null transform. Everything else transforms with 0.
    let null_transform = matches!(&table.tag, b"glyf" | b"loca");
    let flags = 63 | if null_transform { 3 << 6 } else { 0 };
    directory.push(flags);
    directory.extend_from_slice(&table.tag);
    write_base128(&mut directory, table.data.len() as u32)?;
  }

  // Tables are concatenated with no padding between them.
  let mut payload = Vec::new();
  for table in &tables {
    payload.extend_from_slice(&table.data);
  }

  let mut compressed = Vec::new();
  {
    let mut brotli = brotli::CompressorWriter::new(&mut compressed, 4096, 11, 22);
    brotli.write_all(&payload)?;
  }

  let header = 48;
  let length = header + directory.len() + compressed.len();

  let mut out = Vec::with_capacity(length);
  out.extend_from_slice(&WOFF2.to_be_bytes());
  out.extend_from_slice(&flavor.to_be_bytes());
  out.extend_from_slice(&(length as u32).to_be_bytes());
  out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
  out.extend_from_slice(&0u16.to_be_bytes()); // reserved
  out.extend_from_slice(&sfnt_size(&tables).to_be_bytes());
  out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
  out.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
  out.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
  for _ in 0..5 {
    out.extend_from_slice(&0u32.to_be_bytes()); // no metadata, no private block
  }
  out.extend_from_slice(&directory);
  out.extend_from_slice(&compressed);

  Ok(out)
}

/// WOFF2's variable-length integer: seven bits per byte, most significant
/// first, every byte but the last carrying a continuation bit.
fn write_base128(out: &mut Vec<u8>, value: u32) -> Result<()> {
  let mut septets = [0u8; 5];
  let mut count = 0;
  let mut left = value;
  loop {
    septets[count] = (left & 0x7f) as u8;
    count += 1;
    left >>= 7;
    if left == 0 {
      break;
    }
  }
  if count > 5 {
    bail!("table is too large to describe in a WOFF2 directory");
  }
  for i in (0..count).rev() {
    let continues = if i == 0 { 0 } else { 0x80 };
    out.push(septets[i] | continues);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Read as _;

  /// The example set, built the way the CLI builds it.
  pub fn sample_font() -> Vec<u8> {
    let mut icons = Vec::new();
    for (name, code) in [("check", '\u{e900}'), ("ring", '\u{e901}')] {
      let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                      <path d="M4 12l5 5L20 6" fill="none" stroke="#000" stroke-width="2"/>
                    </svg>"##;
      icons.push(crate::font::Icon {
        name: name.to_string(),
        label: name.to_string(),
        group: None,
        source: name.into(),
        codepoint: code,
        outline: crate::svg::parse(svg.as_bytes(), name).unwrap(),
      });
    }
    crate::font::build(&icons, "Probe").unwrap()
  }

  #[test]
  fn woff_carries_every_table_the_font_had() {
    let sfnt = sample_font();
    let (flavor, original) = tables(&sfnt).unwrap();
    let wrapped = woff(&sfnt).unwrap();

    assert_eq!(&wrapped[0..4], b"wOFF");
    assert_eq!(
      u32::from_be_bytes(wrapped[4..8].try_into().unwrap()),
      flavor
    );
    assert_eq!(
      u32::from_be_bytes(wrapped[8..12].try_into().unwrap()) as usize,
      wrapped.len(),
      "the declared length must match the file"
    );
    assert_eq!(
      u16::from_be_bytes(wrapped[12..14].try_into().unwrap()) as usize,
      original.len()
    );

    // Every table must come back out byte for byte.
    for (i, table) in original.iter().enumerate() {
      let entry = 44 + i * 20;
      let offset = u32::from_be_bytes(wrapped[entry + 4..entry + 8].try_into().unwrap()) as usize;
      let comp = u32::from_be_bytes(wrapped[entry + 8..entry + 12].try_into().unwrap()) as usize;
      let orig = u32::from_be_bytes(wrapped[entry + 12..entry + 16].try_into().unwrap()) as usize;
      assert_eq!(&wrapped[entry..entry + 4], &table.tag);
      assert_eq!(orig, table.data.len());

      let stored = &wrapped[offset..offset + comp];
      let recovered = if comp == orig {
        stored.to_vec()
      } else {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(stored)
          .read_to_end(&mut out)
          .unwrap();
        out
      };
      assert_eq!(
        recovered, table.data,
        "table {:?} did not survive",
        table.tag
      );
    }
  }

  #[test]
  fn woff2_brotli_stream_decompresses_to_every_table_in_order() {
    let sfnt = sample_font();
    let (flavor, original) = tables(&sfnt).unwrap();
    let wrapped = woff2(&sfnt).unwrap();

    assert_eq!(&wrapped[0..4], b"wOF2");
    assert_eq!(
      u32::from_be_bytes(wrapped[4..8].try_into().unwrap()),
      flavor
    );
    assert_eq!(
      u32::from_be_bytes(wrapped[8..12].try_into().unwrap()) as usize,
      wrapped.len()
    );

    let compressed_len = u32::from_be_bytes(wrapped[20..24].try_into().unwrap()) as usize;
    let stream = &wrapped[wrapped.len() - compressed_len..];
    let mut payload = Vec::new();
    brotli::Decompressor::new(stream, 4096)
      .read_to_end(&mut payload)
      .unwrap();

    let expected: Vec<u8> = original.iter().flat_map(|t| t.data.clone()).collect();
    assert_eq!(payload, expected);
  }

  #[test]
  fn woff2_is_smaller_than_woff_which_is_smaller_than_the_font() {
    let sfnt = sample_font();
    let woff = woff(&sfnt).unwrap().len();
    let woff2 = woff2(&sfnt).unwrap().len();
    assert!(woff < sfnt.len(), "woff {woff} vs ttf {}", sfnt.len());
    assert!(woff2 < woff, "woff2 {woff2} vs woff {woff}");
  }
}
