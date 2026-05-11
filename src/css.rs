//! The stylesheet that makes the generated font usable from HTML.

use crate::font::Icon;

/// Render the stylesheet for `icons`.
///
/// `font_url` is what goes into `src: url(...)`, and is normally just the font
/// file's name so that the CSS works wherever the two files are copied to
/// together.
pub fn render(icons: &[Icon], family: &str, prefix: &str, font_url: &str) -> String {
    let mut css = String::new();

    css.push_str(&format!(
        "@font-face {{\n  \
           font-family: '{family}';\n  \
           src: url('{font_url}') format('truetype');\n  \
           font-weight: normal;\n  \
           font-style: normal;\n  \
           font-display: block;\n\
         }}\n\n"
    ));

    // Match both `class=\"icon-foo\"` and `class=\"btn icon-foo\"` so the base
    // rules apply without having to also write the bare `.icon` class.
    css.push_str(&format!(
        "[class^=\"{prefix}-\"],\n\
         [class*=\" {prefix}-\"] {{\n  \
           font-family: '{family}' !important;\n  \
           font-style: normal;\n  \
           font-weight: normal;\n  \
           font-variant: normal;\n  \
           text-transform: none;\n  \
           line-height: 1;\n  \
           speak: never;\n  \
           -webkit-font-smoothing: antialiased;\n  \
           -moz-osx-font-smoothing: grayscale;\n\
         }}\n\n"
    ));

    for icon in icons {
        css.push_str(&format!(
            ".{prefix}-{name}::before {{\n  content: \"\\{code:x}\";\n}}\n",
            name = icon.name,
            code = icon.codepoint as u32,
        ));
    }

    css
}
