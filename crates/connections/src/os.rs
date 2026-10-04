//! Operating systems a server can run, how to recognise them and how they are
//! drawn in the sidebar.
//!
//! The glyphs are simplified marks on a 24×24 grid, not the vendors' logos;
//! the colours are the vendors' brand colours. Systems without a mark of
//! their own get a monogram badge in their colour.

/// One entry of the catalog.
#[derive(Debug, PartialEq, Eq)]
pub struct Os {
    /// Stored in profiles; the `ID` of `os-release` where there is one.
    pub id: &'static str,
    pub name: &'static str,
    /// The brand colour, as `#RRGGBB`.
    pub color: &'static str,
    pub(crate) glyph: Glyph,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    /// SVG elements; `{c}` is replaced with the colour it is drawn in.
    Mark(&'static str),
    /// One to three capital letters on a shape.
    Badge(Shape, &'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Circle,
    Square,
    Hexagon,
    Shield,
}

mod catalog;
use Glyph::{Badge, Mark};
use Shape::{Circle, Hexagon, Shield, Square};
pub use catalog::CATALOG;

/// The catalog entry for `id`.
pub fn find(id: &str) -> Option<&'static Os> {
    CATALOG.iter().find(|os| os.id == id)
}

/// The system an `os-release` file describes: its `ID` when the catalog knows
/// it, else the first `ID_LIKE` it knows, else plain Linux.
pub fn from_os_release(text: &str) -> Option<&'static Os> {
    let mut id = None;
    let mut like = None;
    let mut variant = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        match key.trim() {
            "ID" => id = Some(value),
            "ID_LIKE" => like = Some(value),
            "VARIANT_ID" => variant = Some(value),
            _ => {}
        }
    }
    let mut id = id?;
    if id == "fedora" && variant.as_deref() == Some("coreos") {
        id = "fedora-coreos".to_owned();
    }
    std::iter::once(id.as_str())
        .chain(like.iter().flat_map(|like| like.split_whitespace()))
        .find_map(by_release_id)
        .or_else(|| find("linux"))
}

/// Maps `os-release` IDs onto catalog entries; distributions with several IDs
/// share one entry.
fn by_release_id(id: &str) -> Option<&'static Os> {
    let id = match id {
        "opensuse-leap" | "opensuse-tumbleweed" | "opensuse-microos" | "opensuse-slowroll" => {
            "opensuse"
        }
        "sles" | "sled" | "sle-micro" => "suse",
        "archarm" | "arch32" => "arch",
        "manjaro-arm" => "manjaro",
        "redhat" | "rhel" | "rhcos" => "rhel",
        "centos-stream" => "centos",
        "raspberrypi" => "raspbian",
        "fedora-asahi-remix" => "fedora",
        "mariner" => "azurelinux",
        "alt" => "altlinux",
        "hassos" => "haos",
        "cachyos-linux" => "cachyos",
        "chromiumos" => "chromeos",
        id => id,
    };
    find(id)
}

/// Whether `color` is a colour the sidebar can draw: `#RGB` or `#RRGGBB`.
pub fn is_valid_color(color: &str) -> bool {
    color
        .strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `color` as `0xRRGGBB`, if it is valid.
pub fn rgb(color: &str) -> Option<u32> {
    if !is_valid_color(color) {
        return None;
    }
    let hex = &color[1..];
    let hex = if hex.len() == 3 {
        hex.chars().flat_map(|digit| [digit, digit]).collect()
    } else {
        hex.to_owned()
    };
    u32::from_str_radix(&hex, 16).ok()
}

/// A standalone SVG document of `os`, drawn in `color`.
pub fn svg(os: &Os, color: &str) -> Vec<u8> {
    debug_assert!(is_valid_color(color));
    let body = match os.glyph {
        Mark(glyph) => glyph.replace("{c}", color),
        Badge(shape, letters) => badge(shape, letters, color),
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">{body}</svg>"#
    )
    .into_bytes()
}

fn badge(shape: Shape, letters: &str, color: &str) -> String {
    let background = match shape {
        Circle => format!(r#"<circle cx="12" cy="12" r="10.5" fill="{color}"/>"#),
        Square => {
            format!(r#"<rect x="1.5" y="1.5" width="21" height="21" rx="5" fill="{color}"/>"#)
        }
        Hexagon => format!(r#"<path d="M12 1 21.5 6.5v11L12 23 2.5 17.5v-11Z" fill="{color}"/>"#),
        Shield => format!(
            r#"<path d="M12 1.5 21 4.5v7c0 5.5-3.8 9.6-9 11-5.2-1.4-9-5.5-9-11v-7Z" fill="{color}"/>"#
        ),
    };
    // Letters are strokes in an 8×12 box, scaled to fit side by side.
    let count = letters.chars().count().clamp(1, 3);
    let (scale, stroke) = [(0.8, 2.2), (0.62, 1.55), (0.45, 1.25)][count - 1];
    let advance = 8.0 + 2.6;
    let width = (count as f32 * advance - 2.6) * scale;
    let (left, top) = (12.0 - width / 2.0, 12.0 - 6.0 * scale);
    let ink = if is_light(color) {
        "#1F1F1F"
    } else {
        "#FFFFFF"
    };
    let strokes: String = letters
        .chars()
        .take(3)
        .enumerate()
        .map(|(ix, letter)| {
            format!(
                r#"<path transform="translate({x} {top}) scale({scale})" d="{d}"/>"#,
                x = left + ix as f32 * advance * scale,
                d = letter_strokes(letter),
            )
        })
        .collect();
    format!(
        r#"{background}<g fill="none" stroke="{ink}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round">{strokes}</g>"#,
        width = stroke / scale,
    )
}

/// Whether dark ink reads better than white on `color`.
fn is_light(color: &str) -> bool {
    let rgb = rgb(color).unwrap_or(0);
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    0.299 * channel(16) + 0.587 * channel(8) + 0.114 * channel(0) > 0.65
}

/// A capital letter as strokes in an 8×12 box.
fn letter_strokes(letter: char) -> &'static str {
    match letter {
        'A' => "M0 12 4 0l4 12M1.5 7.5h5",
        'B' => "M0 6h4.5Q7 6 7 3T4.5 0H0v12h5q3 0 3-3T5 6",
        'C' => "M8 1.5Q6.5 0 4 0 0 0 0 6t4 6q2.5 0 4-1.5",
        'D' => "M0 0v12h3q5 0 5-6T3 0Z",
        'E' => "M8 0H0v12h8M0 6h6",
        'F' => "M8 0H0v12M0 6h6",
        'G' => "M8 1.5Q6.5 0 4 0 0 0 0 6t4 6q4 0 4-5H4.5",
        'H' => "M0 0v12M8 0v12M0 6h8",
        'I' => "M4 0v12M1 0h6M1 12h6",
        'J' => "M7 0v8.5q0 3.5-3.5 3.5T.5 9",
        'K' => "M0 0v12M8 0 0 7.5M3 5l5 7",
        'L' => "M0 0v12h8",
        'M' => "M0 12V0l4 7 4-7v12",
        'N' => "M0 12V0l8 12V0",
        'O' => "M4 0Q0 0 0 6t4 6 4-6-4-6Z",
        'P' => "M0 12V0h4.5Q8 0 8 3.5T4.5 7H0",
        'Q' => "M4 0Q0 0 0 6t4 6 4-6-4-6ZM5 9l3 3.5",
        'R' => "M0 12V0h4.5Q8 0 8 3.5T4.5 7H0M4 7l4 5",
        'S' => "M8 1.5Q6.5 0 4 0 .5 0 .5 3q0 2.5 3.5 3t3.5 3q0 3-3.5 3-2.5 0-4-1.5",
        'T' => "M0 0h8M4 0v12",
        'U' => "M0 0v8q0 4 4 4t4-4V0",
        'V' => "M0 0l4 12 4-12",
        'W' => "M0 0l2 12 2-8 2 8 2-12",
        'X' => "M0 0l8 12M8 0 0 12",
        'Y' => "M0 0l4 6 4-6M4 6v6",
        'Z' => "M0 0h8L0 12h8",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_colors_valid() {
        let mut ids: Vec<_> = CATALOG.iter().map(|os| os.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        for os in CATALOG {
            assert!(is_valid_color(os.color), "{}", os.id);
            if let Badge(_, letters) = os.glyph {
                assert!((1..=3).contains(&letters.len()), "{}", os.id);
                assert!(
                    letters.chars().all(|c| !letter_strokes(c).is_empty()),
                    "{}",
                    os.id
                );
            }
            let svg = String::from_utf8(svg(os, os.color)).unwrap();
            assert!(!svg.contains("{c}"), "{}", os.id);
            assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        }
    }

    #[test]
    fn os_release_id_wins_then_id_like_then_generic_linux() {
        let ubuntu =
            "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";
        assert_eq!(from_os_release(ubuntu).unwrap().id, "ubuntu");
        let derived = "ID=\"bodhi\"\nID_LIKE=\"ubuntu debian\"\n";
        assert_eq!(from_os_release(derived).unwrap().id, "ubuntu");
        let leap = "ID=\"opensuse-leap\"\nID_LIKE=\"suse opensuse\"";
        assert_eq!(from_os_release(leap).unwrap().id, "opensuse");
        let alma = "ID=\"almalinux\"\nID_LIKE=\"rhel centos fedora\"";
        assert_eq!(from_os_release(alma).unwrap().id, "almalinux");
        let coreos = "ID=fedora\nVARIANT_ID=coreos\n";
        assert_eq!(from_os_release(coreos).unwrap().id, "fedora-coreos");
        let alt = "ID=altlinux\n";
        assert_eq!(from_os_release(alt).unwrap().id, "altlinux");
        let unknown = "ID=exotic\n";
        assert_eq!(from_os_release(unknown).unwrap().id, "linux");
        assert_eq!(from_os_release("# no id\nNAME=x"), None);
        assert_eq!(from_os_release("  ID = 'Arch'  ").unwrap().id, "arch");
    }

    #[test]
    fn badges_pick_readable_ink() {
        let dark = String::from_utf8(svg(find("truenas").unwrap(), "#0095D5")).unwrap();
        assert!(dark.contains(r##"stroke="#FFFFFF""##), "{dark}");
        let light = String::from_utf8(svg(find("guix").unwrap(), "#FFCC00")).unwrap();
        assert!(light.contains(r##"stroke="#1F1F1F""##), "{light}");
    }

    #[test]
    fn colors_are_validated() {
        for valid in ["#fff", "#E95420", "#0d597f"] {
            assert!(is_valid_color(valid), "{valid}");
        }
        for invalid in ["", "fff", "#ffff", "#12345g", "red", "#E95420\"/><script"] {
            assert!(!is_valid_color(invalid), "{invalid}");
            assert_eq!(rgb(invalid), None);
        }
        assert_eq!(rgb("#E95420"), Some(0xE95420));
        assert_eq!(rgb("#0af"), Some(0x00AAFF));
    }
}
