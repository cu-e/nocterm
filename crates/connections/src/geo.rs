//! Embedded fallback flags belong to presentation, not the native adapter.

pub(crate) fn is_country_code(code: &str) -> bool {
    nocterm_connections_model::observation::CountryCode::new(code).is_ok()
}

/// Flags drawn from stripes alone, for when the network is unavailable.
pub(crate) fn builtin_flag(code: &str) -> Option<Vec<u8>> {
    // Whether the stripes are vertical, and their colours from the top or left.
    let (vertical, colors): (bool, &[&str]) = match code {
        "at" => (false, &["#C8102E", "#FFFFFF", "#C8102E"]),
        "be" => (true, &["#000000", "#FDDA24", "#EF3340"]),
        "bg" => (false, &["#FFFFFF", "#00966E", "#D62612"]),
        "de" => (false, &["#000000", "#DD0000", "#FFCE00"]),
        "ee" => (false, &["#0072CE", "#000000", "#FFFFFF"]),
        "fr" => (true, &["#002654", "#FFFFFF", "#CE1126"]),
        "hu" => (false, &["#CE2939", "#FFFFFF", "#477050"]),
        "id" => (false, &["#CE1126", "#FFFFFF"]),
        "ie" => (true, &["#169B62", "#FFFFFF", "#FF883E"]),
        "it" => (true, &["#009246", "#FFFFFF", "#CE2B37"]),
        "lt" => (false, &["#FDB913", "#006A44", "#C1272D"]),
        "lu" => (false, &["#EA141D", "#FFFFFF", "#51ADDA"]),
        "lv" => (
            false,
            &["#9E3039", "#9E3039", "#FFFFFF", "#9E3039", "#9E3039"],
        ),
        "mc" => (false, &["#CE1126", "#FFFFFF"]),
        "ng" => (true, &["#008751", "#FFFFFF", "#008751"]),
        "nl" => (false, &["#AE1C28", "#FFFFFF", "#21468B"]),
        "pl" => (false, &["#FFFFFF", "#DC143C"]),
        "ro" => (true, &["#002B7F", "#FCD116", "#CE1126"]),
        "ru" => (false, &["#FFFFFF", "#0039A6", "#D52B1E"]),
        "ua" => (false, &["#0057B7", "#FFD700"]),
        _ => return None,
    };
    let count = colors.len();
    let stripes: String = colors
        .iter()
        .enumerate()
        .map(|(ix, color)| {
            // Each stripe overlaps the next a little, against seams.
            let (x, y, width, height) = if vertical {
                (ix as f32, 0.0, 1.05, 1.0)
            } else {
                (0.0, ix as f32, 1.0, 1.05)
            };
            format!(r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="{color}"/>"#)
        })
        .collect();
    let view_box = if vertical {
        format!("0 0 {count} 1")
    } else {
        format!("0 0 1 {count}")
    };
    Some(
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="16" viewBox="{view_box}" preserveAspectRatio="none">{stripes}</svg>"#
        )
        .into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_flags_are_svg() {
        for code in ["de", "fr", "nl", "ua", "lv"] {
            let flag = builtin_flag(code).unwrap();
            assert!(String::from_utf8(flag).unwrap().contains("<svg"), "{code}");
        }
        assert!(builtin_flag("us").is_none());
    }
}
