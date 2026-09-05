//! The codepoint manifest, which keeps an icon's codepoint stable across builds.
//!
//! Without it, adding `aardvark.svg` to a set would shift every later icon by
//! one and silently break every page already using the font. The manifest
//! records what each icon was assigned, so a rebuild only ever hands out
//! codepoints to icons it has not seen before.
//!
//! Records are keyed by the icon's path inside the icon folder rather than by
//! its name, because a name is not a stable identity: two files whose names
//! reduce to the same slug are told apart by a number, and that number depends
//! on what else is in the folder. A path does not move when a neighbour
//! appears, so neither does the codepoint.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};

/// Default file name, written inside the icon folder so that it travels with
/// the SVGs it describes rather than with the generated output.
pub const DEFAULT_FILE: &str = "icofon.json";

/// What one build gave to one file.
struct Entry {
  name: String,
  codepoint: char,
}

#[derive(Default)]
pub struct Manifest {
  /// Icon path, relative to the icon folder and always forward-slashed, to
  /// what that file was given. Entries for icons that have since been deleted
  /// are kept, so that a codepoint is never handed to a different icon.
  icons: BTreeMap<String, Entry>,
  /// Name-keyed records, which is how icofon 0.3 and earlier wrote the file.
  /// They are read so that an existing project keeps its codepoints, and each
  /// one is dropped as soon as the icon it describes is written back under its
  /// path. Entries that match no file stay, which is what keeps a deleted
  /// icon's codepoint reserved — and gives it back if the icon returns.
  legacy: BTreeMap<String, char>,
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

    let hex = |hex: &str, what: &str| -> Result<char> {
      u32::from_str_radix(hex, 16)
        .ok()
        .and_then(char::from_u32)
        .with_context(|| {
          format!(
            "{} lists '{hex}' for {what}, which is not a codepoint",
            path.display()
          )
        })
    };

    let mut icons = BTreeMap::new();
    for (key, record) in parsed.icons {
      let codepoint = hex(&record.codepoint, &format!("'{key}'"))?;
      icons.insert(
        key,
        Entry {
          name: record.name,
          codepoint,
        },
      );
    }

    let mut legacy = BTreeMap::new();
    for (name, value) in parsed.codepoints {
      legacy.insert(name.clone(), hex(&value, &format!("'{name}'"))?);
    }

    let mut retired = BTreeSet::new();
    for value in parsed.retired {
      retired.insert(hex(&value, "a retired slot")?);
    }
    Ok(Self {
      icons,
      legacy,
      retired,
    })
  }

  /// The codepoint recorded for the icon at `key`.
  ///
  /// Falls back to the name-keyed record an older icofon wrote, so the first
  /// build after an upgrade keeps every codepoint exactly where it was.
  pub fn get(&self, key: &str, name: &str) -> Option<char> {
    match self.icons.get(key) {
      Some(entry) => Some(entry.codepoint),
      None => self.legacy.get(name).copied(),
    }
  }

  /// The name the icon at `key` was given last build.
  ///
  /// Used to keep a numbered name attached to the file that earned it, rather
  /// than to whichever file happens to sort into that position now.
  pub fn name(&self, key: &str) -> Option<&str> {
    self.icons.get(key).map(|entry| entry.name.as_str())
  }

  /// Record what the icon at `key` was assigned this build.
  ///
  /// This overwrites an earlier record rather than refusing to: a `uE9F0-`
  /// prefix on the file name is an explicit instruction to move that icon, and
  /// the manifest's job is to remember the decision, not to veto it. The
  /// invariant that matters — two icons never sharing a codepoint — is
  /// enforced while codepoints are being assigned.
  ///
  /// The codepoint an icon vacates by moving is retired rather than freed, so
  /// that it is not quietly recycled to an unrelated icon on the next build.
  pub fn insert(&mut self, key: &str, name: &str, codepoint: char) {
    let entry = Entry {
      name: name.to_string(),
      codepoint,
    };
    if let Some(previous) = self.icons.insert(key.to_string(), entry)
      && previous.codepoint != codepoint
    {
      self.retired.insert(previous.codepoint);
    }
    // This icon is now recorded under its path, so any name-keyed record of it
    // has been migrated and would only reserve the same codepoint twice.
    self.legacy.remove(name);
    self.legacy.retain(|_, value| *value != codepoint);
    // A codepoint that is live again is no longer retired.
    self.retired.remove(&codepoint);
  }

  /// The name of the icon the record currently gives `codepoint` to, if any.
  ///
  /// A retired codepoint has no holder: it is reserved so it is never handed
  /// out again, but nothing is using it, so nothing clashes with taking it back
  /// by name.
  pub fn holder(&self, codepoint: char) -> Option<&str> {
    self
      .icons
      .values()
      .find(|entry| entry.codepoint == codepoint)
      .map(|entry| entry.name.as_str())
      .or_else(|| {
        self
          .legacy
          .iter()
          .find(|(_, held)| **held == codepoint)
          .map(|(name, _)| name.as_str())
      })
  }

  /// Every codepoint the manifest has ever handed out, including to icons
  /// that are no longer in the folder and to slots vacated by a move.
  pub fn reserved(&self) -> impl Iterator<Item = char> + '_ {
    self
      .icons
      .values()
      .map(|entry| entry.codepoint)
      .chain(self.legacy.values().copied())
      .chain(self.retired.iter().copied())
  }

  pub fn save(&self, path: &Path) -> Result<()> {
    let document = Document {
      icons: self
        .icons
        .iter()
        .map(|(key, entry)| {
          (
            key.clone(),
            Record {
              name: entry.name.clone(),
              codepoint: hex(entry.codepoint),
            },
          )
        })
        .collect(),
      codepoints: self
        .legacy
        .iter()
        .map(|(name, cp)| (name.clone(), hex(*cp)))
        .collect(),
      retired: self.retired.iter().map(|cp| hex(*cp)).collect(),
    };
    let mut text = serde_json::to_string_pretty(&document).context("serialising the manifest")?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
  }
}

fn hex(codepoint: char) -> String {
  format!("{:04x}", codepoint as u32)
}

/// On-disk shape. A `BTreeMap` keeps the file sorted, so diffs stay readable
/// when icons are added.
#[derive(serde::Serialize, serde::Deserialize)]
struct Document {
  /// Path-keyed records, which is what a build writes.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  icons: BTreeMap<String, Record>,
  /// Name-keyed records left over from icofon 0.3 and earlier, dropped one by
  /// one as the icons they describe are rewritten under their paths.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  codepoints: BTreeMap<String, String>,
  /// Absent in manifests written before retirement was tracked, and omitted
  /// again once nothing is retired.
  #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
  retired: BTreeSet<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
  name: String,
  codepoint: String,
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
    assert!(manifest.get("anything.svg", "anything").is_none());
  }

  #[test]
  fn round_trips_through_disk() {
    let path = temp("roundtrip");
    let mut manifest = Manifest::default();
    manifest.insert("check.svg", "check", '\u{e900}');
    manifest.insert("star.svg", "star", '\u{e9f0}');
    manifest.save(&path).unwrap();

    let reloaded = Manifest::load(&path).unwrap();
    assert_eq!(reloaded.get("check.svg", "check"), Some('\u{e900}'));
    assert_eq!(reloaded.get("star.svg", "star"), Some('\u{e9f0}'));
    assert_eq!(reloaded.name("star.svg"), Some("star"));
    std::fs::remove_file(&path).ok();
  }

  #[test]
  fn a_record_follows_the_file_rather_than_the_name() {
    // The name is what a clashing neighbour can change; the path is not. A
    // renamed icon must keep the codepoint its file already had.
    let mut manifest = Manifest::default();
    manifest.insert("map_pin.svg", "map-pin-2", '\u{e901}');
    assert_eq!(manifest.get("map_pin.svg", "map-pin-3"), Some('\u{e901}'));
  }

  #[test]
  fn a_later_assignment_overwrites_the_record() {
    // Moving an icon with a uE901- prefix is deliberate; the manifest
    // follows the decision instead of blocking it.
    let mut manifest = Manifest::default();
    manifest.insert("check.svg", "check", '\u{e900}');
    manifest.insert("check.svg", "check", '\u{e901}');
    assert_eq!(manifest.get("check.svg", "check"), Some('\u{e901}'));
  }

  #[test]
  fn a_vacated_codepoint_is_retired_rather_than_freed() {
    let mut manifest = Manifest::default();
    manifest.insert("check.svg", "check", '\u{e900}');
    manifest.insert("check.svg", "check", '\u{e950}');

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
    manifest.insert("check.svg", "check", '\u{e900}');
    manifest.insert("check.svg", "check", '\u{e950}');
    manifest.insert("check.svg", "check", '\u{e900}');
    assert_eq!(manifest.reserved().filter(|c| *c == '\u{e900}').count(), 1);
  }

  #[test]
  fn retirement_survives_a_round_trip() {
    let path = temp("retired");
    let mut manifest = Manifest::default();
    manifest.insert("check.svg", "check", '\u{e900}');
    manifest.insert("check.svg", "check", '\u{e950}');
    manifest.save(&path).unwrap();

    let reloaded = Manifest::load(&path).unwrap();
    assert!(reloaded.reserved().any(|c| c == '\u{e900}'));
    std::fs::remove_file(&path).ok();
  }

  #[test]
  fn deleted_icons_keep_their_codepoints_reserved() {
    let mut manifest = Manifest::default();
    manifest.insert("retired.svg", "retired", '\u{e900}');
    assert!(manifest.reserved().any(|c| c == '\u{e900}'));
  }

  #[test]
  fn a_name_keyed_manifest_is_read_and_migrated() {
    let path = temp("legacy");
    std::fs::write(
      &path,
      r#"{ "codepoints": { "check": "e900", "gone": "e901" } }"#,
    )
    .unwrap();

    let mut manifest = Manifest::load(&path).unwrap();
    // The old record is what an existing project's codepoints hang on, so it
    // has to answer for a file it has never seen under its path.
    assert_eq!(manifest.get("check.svg", "check"), Some('\u{e900}'));
    // And it keeps reserving the codepoint of an icon that has since gone.
    assert!(manifest.reserved().any(|c| c == '\u{e901}'));

    manifest.insert("check.svg", "check", '\u{e900}');
    manifest.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("check.svg"), "{text}");
    assert!(
      !text.contains("\"check\": \"e900\""),
      "the migrated record is not kept twice: {text}"
    );
    assert!(
      text.contains("\"gone\": \"e901\""),
      "an unmatched record stays, so the icon can come back: {text}"
    );
    std::fs::remove_file(&path).ok();
  }
}
