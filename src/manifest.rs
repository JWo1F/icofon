//! The codepoint manifest, which keeps an icon's codepoint stable across builds.
//!
//! Without it, adding `aardvark.svg` to a set would shift every later icon by
//! one and silently break every page already using the font. The manifest
//! records what each icon was assigned, so a rebuild only ever hands out
//! codepoints to icons it has not seen before.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};

/// Default file name, written inside the icon folder so that it travels with
/// the SVGs it describes rather than with the generated output.
pub const DEFAULT_FILE: &str = "icofon.json";

#[derive(Default)]
pub struct Manifest {
  /// Icon name to codepoint. Entries for icons that have since been deleted
  /// are kept, so that a codepoint is never handed to a different icon.
  codepoints: BTreeMap<String, char>,
  /// Codepoints that were live under an earlier build but no longer belong to
  /// any icon, because the icon that held them was pinned somewhere else.
  /// Keeping them means a codepoint is never recycled, whether the icon that
  /// held it was deleted or moved.
  retired: BTreeSet<char>,
}

impl Manifest {
  /// Read the manifest at `path`. A missing file is an empty manifest, which
  /// is what the first build of a new icon set sees.
  pub fn load(path: &Path) -> Result<Self> {
    let text = match std::fs::read_to_string(path) {
      Ok(text) => text,
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
        return Ok(Self::default());
      }
      Err(error) => {
        return Err(error).with_context(|| format!("reading {}", path.display()));
      }
    };

    let parsed: Document =
      serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

    let mut codepoints = BTreeMap::new();
    for (name, hex) in parsed.codepoints {
      let codepoint = u32::from_str_radix(&hex, 16)
        .ok()
        .and_then(char::from_u32)
        .with_context(|| {
          format!(
            "{} lists '{hex}' for '{name}', which is not a codepoint",
            path.display()
          )
        })?;
      codepoints.insert(name, codepoint);
    }

    let mut retired = BTreeSet::new();
    for hex in parsed.retired {
      let codepoint = u32::from_str_radix(&hex, 16)
        .ok()
        .and_then(char::from_u32)
        .with_context(|| {
          format!(
            "{} retires '{hex}', which is not a codepoint",
            path.display()
          )
        })?;
      retired.insert(codepoint);
    }
    Ok(Self {
      codepoints,
      retired,
    })
  }

  pub fn get(&self, name: &str) -> Option<char> {
    self.codepoints.get(name).copied()
  }

  /// Record what `name` was assigned this build.
  ///
  /// This overwrites an earlier record rather than refusing to: a `uE9F0-`
  /// prefix on the file name is an explicit instruction to move that icon, and
  /// the manifest's job is to remember the decision, not to veto it. The
  /// invariant that matters — two icons never sharing a codepoint — is
  /// enforced while codepoints are being assigned.
  ///
  /// The codepoint an icon vacates by moving is retired rather than freed, so
  /// that it is not quietly recycled to an unrelated icon on the next build.
  pub fn insert(&mut self, name: &str, codepoint: char) {
    if let Some(previous) = self.codepoints.insert(name.to_string(), codepoint)
      && previous != codepoint
    {
      self.retired.insert(previous);
    }
    // A codepoint that is live again is no longer retired.
    self.retired.remove(&codepoint);
  }

  /// Every codepoint the manifest has ever handed out, including to icons
  /// that are no longer in the folder and to slots vacated by a move.
  pub fn reserved(&self) -> impl Iterator<Item = char> + '_ {
    self
      .codepoints
      .values()
      .copied()
      .chain(self.retired.iter().copied())
  }

  pub fn save(&self, path: &Path) -> Result<()> {
    let document = Document {
      codepoints: self
        .codepoints
        .iter()
        .map(|(name, cp)| (name.clone(), format!("{:04x}", *cp as u32)))
        .collect(),
      retired: self
        .retired
        .iter()
        .map(|cp| format!("{:04x}", *cp as u32))
        .collect(),
    };
    let mut text = serde_json::to_string_pretty(&document).context("serialising the manifest")?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
  }
}

/// On-disk shape. A `BTreeMap` keeps the file sorted, so diffs stay readable
/// when icons are added.
#[derive(serde::Serialize, serde::Deserialize)]
struct Document {
  codepoints: BTreeMap<String, String>,
  /// Absent in manifests written before retirement was tracked, and omitted
  /// again once nothing is retired, so the common file stays a single map.
  #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
  retired: BTreeSet<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("icofon-manifest-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(DEFAULT_FILE)
  }

  #[test]
  fn a_missing_file_is_an_empty_manifest() {
    let manifest = Manifest::load(Path::new("/nonexistent/icofon.json")).unwrap();
    assert!(manifest.get("anything").is_none());
  }

  #[test]
  fn round_trips_through_disk() {
    let path = temp("roundtrip");
    let mut manifest = Manifest::default();
    manifest.insert("check", '\u{e900}');
    manifest.insert("star", '\u{e9f0}');
    manifest.save(&path).unwrap();

    let reloaded = Manifest::load(&path).unwrap();
    assert_eq!(reloaded.get("check"), Some('\u{e900}'));
    assert_eq!(reloaded.get("star"), Some('\u{e9f0}'));
    std::fs::remove_file(&path).ok();
  }

  #[test]
  fn a_later_assignment_overwrites_the_record() {
    // Moving an icon with a uE901- prefix is deliberate; the manifest
    // follows the decision instead of blocking it.
    let mut manifest = Manifest::default();
    manifest.insert("check", '\u{e900}');
    manifest.insert("check", '\u{e901}');
    assert_eq!(manifest.get("check"), Some('\u{e901}'));
  }

  #[test]
  fn a_vacated_codepoint_is_retired_rather_than_freed() {
    let mut manifest = Manifest::default();
    manifest.insert("check", '\u{e900}');
    manifest.insert("check", '\u{e950}');

    let reserved: Vec<_> = manifest.reserved().collect();
    assert!(
      reserved.contains(&'\u{e900}'),
      "the vacated slot stays reserved"
    );
    assert!(reserved.contains(&'\u{e950}'));
  }

  #[test]
  fn a_codepoint_that_becomes_live_again_is_un_retired() {
    let mut manifest = Manifest::default();
    manifest.insert("check", '\u{e900}');
    manifest.insert("check", '\u{e950}');
    manifest.insert("check", '\u{e900}');
    assert_eq!(manifest.reserved().filter(|c| *c == '\u{e900}').count(), 1);
  }

  #[test]
  fn retirement_survives_a_round_trip() {
    let path = temp("retired");
    let mut manifest = Manifest::default();
    manifest.insert("check", '\u{e900}');
    manifest.insert("check", '\u{e950}');
    manifest.save(&path).unwrap();

    let reloaded = Manifest::load(&path).unwrap();
    assert!(reloaded.reserved().any(|c| c == '\u{e900}'));
    std::fs::remove_file(&path).ok();
  }

  #[test]
  fn deleted_icons_keep_their_codepoints_reserved() {
    let mut manifest = Manifest::default();
    manifest.insert("retired", '\u{e900}');
    assert!(manifest.reserved().any(|c| c == '\u{e900}'));
  }
}
