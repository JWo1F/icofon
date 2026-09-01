//! The stylesheet that makes the generated font usable from HTML.

mod sheet;

use crate::font::Icon;
use sheet::{Selector, Sheet, Value};

/// How an icon is named in HTML, and so how the stylesheet selects it.
///
/// By default the stylesheet matches any class starting with the prefix, so an
/// icon is a single `icon-arrow-left` class. That is convenient, but it claims
/// the whole `icon-*` namespace: a class of your own that happens to start the
/// same way picks up the icon font too.
///
/// With `base_class` the prefix becomes a class in its own right and an icon is
/// written `class="icon icon-arrow-left"`. Every rule is then scoped to
/// elements carrying both, so nothing outside this font is touched.
#[derive(Clone, Copy)]
pub struct Classes<'a> {
  pub prefix: &'a str,
  pub base_class: bool,
}

impl Classes<'_> {
  /// What to write in a `class` attribute for the icon named `name`.
  pub fn attr(&self, name: &str) -> String {
    let prefix = self.prefix;
    if self.base_class {
      format!("{prefix} {prefix}-{name}")
    } else {
      format!("{prefix}-{name}")
    }
  }

  /// The selector matching an element that carries [`Classes::attr`].
  pub fn selector(&self, name: &str) -> Selector {
    let icon = Selector::class(&format!("{}-{name}", self.prefix));
    if self.base_class {
      Selector::class(self.prefix).and(icon)
    } else {
      icon
    }
  }

  /// The selectors the base font rules hang on.
  ///
  /// With a base class there is one class to hang them on. Without one, match
  /// both `class="icon-foo"` and `class="btn icon-foo"`, so the rules apply
  /// without having to also write the bare `.icon` class.
  fn base_selectors(&self) -> Vec<Selector> {
    let prefix = self.prefix;
    if self.base_class {
      vec![Selector::class(prefix)]
    } else {
      vec![
        Selector::attribute_starts_with("class", &format!("{prefix}-")),
        Selector::attribute_contains("class", &format!(" {prefix}-")),
      ]
    }
  }
}

/// Render the stylesheet for `icons`.
///
/// `font_url` is what goes into `src: url(...)`, and is normally just the font
/// file's name so that the CSS works wherever the two files are copied to
/// together.
pub fn render(icons: &[Icon], family: &str, classes: Classes<'_>, font_url: &str) -> String {
  let mut sheet = Sheet::new();

  sheet.at_rule("font-face", |block| {
    block.set("font-family", Value::quoted(family));
    block.set(
      "src",
      Value::list([
        Value::url(font_url),
        Value::call("format", Value::quoted("truetype")),
      ]),
    );
    block.set("font-weight", Value::keyword("normal"));
    block.set("font-style", Value::keyword("normal"));
    block.set("font-display", Value::keyword("block"));
  });
  sheet.blank_line();

  sheet.rule(classes.base_selectors(), |block| {
    // The family has to win against whatever the surrounding page sets on the
    // element, which is usually a utility class with a higher specificity.
    block.set_important("font-family", Value::quoted(family));
    block.set("font-style", Value::keyword("normal"));
    block.set("font-weight", Value::keyword("normal"));
    block.set("font-variant", Value::keyword("normal"));
    block.set("text-transform", Value::keyword("none"));
    block.set("line-height", Value::keyword("1"));
    block.set("speak", Value::keyword("never"));
    block.set("-webkit-font-smoothing", Value::keyword("antialiased"));
    block.set("-moz-osx-font-smoothing", Value::keyword("grayscale"));
  });
  sheet.blank_line();

  for icon in icons {
    sheet.rule([classes.selector(&icon.name).before()], |block| {
      block.set("content", Value::glyph(icon.codepoint));
    });
  }

  sheet.finish()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::svg;

  fn icon(name: &str, codepoint: char) -> Icon {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
                        <rect width="24" height="24" fill="#000"/>
                      </svg>"##;
    Icon {
      name: name.to_string(),
      label: name.to_string(),
      group: None,
      source: name.into(),
      codepoint,
      outline: svg::parse(svg.as_bytes(), name).unwrap(),
    }
  }

  #[test]
  fn by_default_one_class_carries_both_the_font_and_the_glyph() {
    let classes = Classes {
      prefix: "icon",
      base_class: false,
    };
    let css = render(&[icon("arrow-left", '\u{e900}')], "Icons", classes, "f.ttf");

    assert!(css.contains(r#"[class^="icon-"]"#));
    assert!(css.contains(r#"[class*=" icon-"]"#));
    assert!(css.contains(".icon-arrow-left::before"));
  }

  #[test]
  fn a_base_class_scopes_every_rule_to_icons_from_this_font() {
    let classes = Classes {
      prefix: "icon",
      base_class: true,
    };
    let css = render(&[icon("arrow-left", '\u{e900}')], "Icons", classes, "f.ttf");

    // Nothing is claimed by name alone, so a class of the user's own that
    // happens to start `icon-` is left entirely untouched.
    assert!(!css.contains("[class"));
    assert!(css.contains(".icon {\n"));
    assert!(css.contains(".icon.icon-arrow-left::before"));
  }

  #[test]
  fn a_family_name_cannot_break_out_of_the_declaration_it_sits_in() {
    let classes = Classes {
      prefix: "icon",
      base_class: false,
    };
    // A quote would otherwise end the string, and the `}` would end the rule,
    // leaving whatever follows to be read as CSS of its own.
    let hostile = "My ' Icons; } body{display:none";
    let css = render(&[icon("check", '\u{e900}')], hostile, classes, "f.ttf");

    assert!(css.contains(r"font-family: 'My \' Icons; } body{display:none';"));
    // One `@font-face`, one base rule, one icon rule -- and nothing else.
    assert_eq!(css.matches('{').count(), css.matches('}').count());
    assert!(!css.contains("body{display:none;"));
  }

  #[test]
  fn markup_and_selector_agree_in_both_modes() {
    let bare = Classes {
      prefix: "ico",
      base_class: false,
    };
    assert_eq!(bare.attr("star"), "ico-star");
    assert_eq!(bare.selector("star").to_string(), ".ico-star");

    let based = Classes {
      prefix: "ico",
      base_class: true,
    };
    assert_eq!(based.attr("star"), "ico ico-star");
    assert_eq!(based.selector("star").to_string(), ".ico.ico-star");
  }
}
