use super::*;
use nocterm_design::DesignTokens;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn accepts_hex_only() {
    for (input, output) in [
        ("#123", "#112233"),
        ("#aBcD", "#aabbccdd"),
        ("#123456", "#123456"),
        ("#ABCdef12", "#abcdef12"),
    ] {
        assert_eq!(parse_color(input).unwrap().to_string(), output);
    }
    for input in ["transparent", "00000000", "123456", "#12345", "#zzzzzz", ""] {
        assert_eq!(parse_color(input), None);
    }
}
#[test]
fn lenient_versions_and_bom() {
    for name in [
        "v0_1_minimal.json",
        "v0_2_comments_trailing_commas.json",
        "dracula.json",
    ] {
        let source = fixture(name);
        assert!(!parse_family(&source).unwrap().themes.is_empty());
        let mut bom = vec![0xef, 0xbb, 0xbf];
        bom.extend(source);
        assert!(parse_family(&bom).is_ok());
    }
}
#[test]
fn tolerant_values_aliases_and_invalid_families() {
    let source = br##"{"name":"F","themes":[{"name":"T","appearance":"dark","style":{"x":null,"y":2,"background":"transparent","scrollbar_thumb.background":"#123","element.selection.background":"#456","players":[{"cursor":"#789","selection":null}]}}]}"##;
    let theme = parse_family(source).unwrap().themes.remove(0);
    assert_eq!(theme.colors.len(), 4);
    assert_eq!(
        theme.colors["scrollbar.thumb.background"],
        parse_color("#123").unwrap()
    );
    assert_eq!(theme.players[0].cursor, parse_color("#789"));
    for source in [b"{}".as_slice(), b"{\"themes\":[]}", b"bad"] {
        assert!(parse_family(source).is_err());
    }
    assert!(parse_family(&vec![b' '; FILE_LIMIT + 1]).is_err());
}
#[test]
fn mapping_sources_fallbacks_contrast_and_translucency() {
    let source = br##"{"themes":[{"name":"Synthetic","appearance":"dark","style":{"editor.background":"#000","background":"#111","text":"#fff","border":"#222","border.variant":"#333","border.focused":"#444","element.background":"#555","surface.background":"#666","text.muted":"#777","text.accent":"#eee","element.hover":"#888","element.active":"#999","ghost_element.hover":"#aaa","ghost_element.selected":"#bbb","elevated_surface.background":"#ccc","panel.background":"#ddd","title_bar.background":"#123","status_bar.background":"#234","tab_bar.background":"#345","tab.inactive_background":"#456","tab.active_background":"#567","scrollbar.track.background":"#678","scrollbar.thumb.background":"#789","scrollbar.thumb.hover_background":"#89a","link_text.hover":"#9ab","drop_target.background":"#abc","error":"#bcd","success":"#cde","warning":"#def","info":"#ef0","terminal.ansi.red":"#f00","players":[{"cursor":"#0f0","selection":"#00f8"}],"terminal.foreground":"#ffffff80","terminal.ansi.blue":"#ffffff80"}}]}"##;
    let theme = parse_family(source).unwrap().themes.remove(0);
    let base = DesignTokens::builtin();
    let palette = theme.palette(&base.dark);
    let ui: BTreeMap<_, _> = palette.ui.iter().collect();
    for (keys, value) in [
        (vec!["background"], "#000"),
        (
            vec![
                "foreground",
                "secondary.foreground",
                "popover.foreground",
                "sidebar.foreground",
                "accent.foreground",
                "tab.active.foreground",
            ],
            "#fff",
        ),
        (
            vec![
                "border",
                "window.border",
                "sidebar.border",
                "title_bar.border",
                "status_bar.border",
            ],
            "#222",
        ),
        (vec!["input.border"], "#333"),
        (vec!["ring", "list.active.border"], "#444"),
        (
            vec![
                "muted.background",
                "secondary.background",
                "tab_bar.segmented.background",
            ],
            "#555",
        ),
        (vec!["muted.foreground", "tab.foreground"], "#777"),
        (vec!["primary.background", "link"], "#eee"),
        (vec!["primary.foreground"], "#000"),
        (vec!["secondary.hover.background"], "#888"),
        (vec!["secondary.active.background"], "#999"),
        (
            vec![
                "accent.background",
                "list.hover.background",
                "sidebar.accent.background",
            ],
            "#aaa",
        ),
        (
            vec!["list.active.background", "sidebar.primary.background"],
            "#bbb",
        ),
        (vec!["selection.background"], "#00f8"),
        (vec!["caret"], "#0f0"),
        (vec!["popover.background"], "#ccc"),
        (vec!["sidebar.background", "list.head.background"], "#ddd"),
        (vec!["title_bar.background"], "#123"),
        (vec!["status_bar.background"], "#234"),
        (vec!["tab_bar.background"], "#345"),
        (vec!["tab.background"], "#456"),
        (vec!["tab.active.background"], "#567"),
        (vec!["scrollbar.background"], "#678"),
        (vec!["scrollbar.thumb.background"], "#789"),
        (vec!["scrollbar.thumb.hover.background"], "#89a"),
        (vec!["link.hover"], "#9ab"),
        (vec!["drop_target.background"], "#abc"),
        (vec!["danger.background"], "#bcd"),
        (vec!["success.background", "base.green"], "#cde"),
        (vec!["warning.background", "base.yellow"], "#def"),
        (vec!["info.background"], "#ef0"),
        (vec!["base.red"], "#f00"),
    ] {
        for key in keys {
            assert_eq!(ui[key], parse_color(value).unwrap(), "{key}");
        }
    }
    assert_eq!(palette.canvas, parse_color("#111"));
    assert_eq!(palette.terminal.background, None);
    assert_eq!(palette.terminal.foreground, Some(Color::rgb(128, 128, 128)));
    assert_eq!(palette.terminal.blue, Color::rgb(128, 128, 128));
    assert_eq!(palette.terminal.cursor, parse_color("#0f0"));
    assert_eq!(palette.terminal.selection, parse_color("#00f8"));
    assert_eq!(
        palette.terminal.bright_white,
        base.dark.terminal.bright_white
    );
}
#[test]
fn dracula_imports_valid_ansi_and_refines_its_invalid_bright_white() {
    let theme = parse_family(&fixture("dracula.json"))
        .unwrap()
        .themes
        .remove(0);
    let palette = theme.palette(&DesignTokens::builtin().dark);
    assert_eq!(
        palette.terminal.background,
        Some(theme.colors["terminal.background"])
    );
    assert_eq!(palette.terminal.red, parse_color("#ff5555").unwrap());
    assert_eq!(palette.terminal.ansi().len(), 16);
    for (name, color) in [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright_black",
        "bright_red",
        "bright_green",
        "bright_yellow",
        "bright_blue",
        "bright_magenta",
        "bright_cyan",
        "bright_white",
    ]
    .into_iter()
    .zip(palette.terminal.ansi())
    {
        if name == "bright_white" {
            // Original Dracula 1.1.2 spells this value "ffffffff" without #.
            assert_eq!(color, DesignTokens::builtin().dark.terminal.bright_white);
        } else {
            assert_eq!(color, theme.colors[&format!("terminal.ansi.{name}")]);
        }
    }
}

#[test]
fn inherited_translucent_terminal_background_and_explicit_alpha_are_opaque() {
    let source=br##"{"themes":[{"name":"Alpha","appearance":"dark","style":{"editor.background":"#ff000080","text":"#ffffff80"}}]}"##;
    let theme = parse_family(source).unwrap().themes.remove(0);
    let base = DesignTokens::builtin();
    let palette = theme.palette(&base.dark);
    assert_eq!(palette.terminal.background.unwrap().a, 255);
    assert_eq!(palette.terminal.foreground.unwrap().a, 255);
    assert!(palette.terminal.ansi().iter().all(|c| c.a == 255));
    let source=br##"{"themes":[{"name":"Alpha","appearance":"light","style":{"editor.background":"#ffffff","terminal.background":"#00000080","terminal.foreground":"#ff000080","terminal.ansi.black":"#00000080"}}]}"##;
    let palette = parse_family(source).unwrap().themes[0].palette(&base.light);
    assert_eq!(palette.terminal.background, Some(Color::rgb(127, 127, 127)));
    assert_eq!(palette.terminal.foreground, Some(Color::rgb(191, 63, 63)));
    assert_eq!(palette.terminal.black, Color::rgb(63, 63, 63));
}

#[test]
fn alternate_sources_and_player_priority_refine_the_palette() {
    let source=br##"{"themes":[{"name":"Fallbacks","appearance":"light","style":{"background":"#123","editor.foreground":"#4568","surface.background":"#234","element.hover":"#345","element.selected":"#456","element.selection.background":"#5678","border":"#678","border.focused":"#789","text.accent":"#abc","terminal.ansi.background":"#fff","terminal.ansi.foreground":"#999","terminal.ansi.magenta":"#f0f","terminal.ansi.cyan":"#0ff","players":[{"cursor":"#def","selection":"#ef08"},{"cursor":"#000","selection":"#000"}]}}]}"##;
    let theme = parse_family(source).unwrap().themes.remove(0);
    let palette = theme.palette(&DesignTokens::builtin().light);
    let ui: BTreeMap<_, _> = palette.ui.iter().collect();
    for (key, value) in [
        ("background", "#123"),
        ("foreground", "#4568"),
        ("muted.background", "#234"),
        ("input.border", "#678"),
        ("sidebar.background", "#234"),
        ("list.head.background", "#234"),
        ("title_bar.background", "#123"),
        ("status_bar.background", "#123"),
        ("accent.background", "#345"),
        ("list.active.background", "#456"),
        ("base.magenta", "#f0f"),
        ("base.cyan", "#0ff"),
        ("caret", "#def"),
        ("primary.background", "#abc"),
        ("selection.background", "#ef08"),
    ] {
        assert_eq!(ui[key], parse_color(value).unwrap(), "{key}");
    }
    assert_eq!(palette.terminal.background, parse_color("#fff"));
    assert_eq!(palette.terminal.foreground, parse_color("#999"));
    assert_eq!(palette.terminal.cursor, parse_color("#def"));
    assert_eq!(palette.terminal.selection, parse_color("#ef08"));
    let source=br##"{"themes":[{"name":"Fallbacks","appearance":"light","style":{"editor.background":"#fff","editor.foreground":"#0008","border.focused":"#123","element.selection_background":"#2348"}}]}"##;
    let palette = parse_family(source).unwrap().themes[0].palette(&DesignTokens::builtin().light);
    let ui: BTreeMap<_, _> = palette.ui.iter().collect();
    assert_eq!(ui["primary.background"], parse_color("#123").unwrap());
    assert_eq!(ui["caret"], parse_color("#123").unwrap());
    assert_eq!(ui["selection.background"], parse_color("#2348").unwrap());
    assert_eq!(palette.terminal.foreground.unwrap().a, 255);
}
