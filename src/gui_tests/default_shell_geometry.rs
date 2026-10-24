//! The default shell geometry and toggle presentation that the layout work must preserve.
use super::*;
use gpui_kit::ElementId;
use gpui_kit::{Bounds, Pixels, SharedString, point, px, size};
use nocterm_ui::{IconName, UiLayout};
use nocterm_workspace::test_support::{
    Toggle, set_side_panel_attention, toggle_id, toggle_presentation,
};

const NAVIGATION: [(&str, IconName); 5] = [
    ("Connections", IconName::Server),
    ("Explorer", IconName::FolderTree),
    ("Snippets", IconName::ScrollText),
    ("Containers", IconName::Container),
    ("Workspaces", IconName::Layers),
];

/// The footer's items from the last frame as `(name, bounds)`, left to right.
///
/// Names are `navigation:<ix>`, `monitor`, `notice`, `local-terminal` and
/// `side-panel`; an item that is not drawn is absent.
fn footer_items(window: &gpui_kit::Window) -> Vec<(String, Bounds<Pixels>)> {
    let mut items = Vec::new();
    for ix in 0.. {
        let Some(item) = window.try_find(toggle_id(Toggle::Navigation(ix))) else {
            break;
        };
        items.push((format!("navigation:{ix}"), item.bounds()));
    }
    for (name, id) in [
        ("monitor", ElementId::from("host-monitor")),
        ("notice", "notice-bar".into()),
        ("local-terminal", toggle_id(Toggle::LocalTerminal)),
        ("side-panel", toggle_id(Toggle::SidePanel)),
    ] {
        if let Some(item) = window.try_find(id) {
            items.push((name.to_owned(), item.bounds()));
        }
    }
    items.sort_by(|a, b| f32::from(a.1.left()).total_cmp(&f32::from(b.1.left())));
    items
}

/// The footer's item names, left to right.
fn footer_ids(window: &gpui_kit::Window) -> Vec<String> {
    footer_items(window)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// The body's columns from the last frame as `(name, bounds)`, left to right:
/// `navigation`, `content` and `side-panel`; a column that is not drawn is absent.
fn columns(window: &gpui_kit::Window) -> Vec<(&'static str, Bounds<Pixels>)> {
    let mut columns = Vec::new();
    for (name, id) in [
        (
            "navigation",
            ElementId::from((ElementId::from("panel-group-column"), "navigation")),
        ),
        ("content", "workspace-content".into()),
        (
            "side-panel",
            ElementId::from((ElementId::from("panel-group-column"), "assistant")),
        ),
    ] {
        if let Some(column) = window.try_find(id) {
            columns.push((name, column.bounds()));
        }
    }
    columns.sort_by(|a, b| f32::from(a.1.left()).total_cmp(&f32::from(b.1.left())));
    columns
}

/// The body's column names, left to right.
fn column_ids(window: &gpui_kit::Window) -> Vec<&'static str> {
    columns(window).into_iter().map(|(name, _)| name).collect()
}

fn shell(
    cx: &mut TestAppContext,
    layout: UiLayout,
    agent: bool,
    width: f32,
) -> (AnyWindowHandle, Entity<Workspace>) {
    shell_on(cx, layout, agent, width, false)
}

/// Like [`shell`]; `local_host` also gives the application a local host to monitor.
fn shell_on(
    cx: &mut TestAppContext,
    layout: UiLayout,
    agent: bool,
    width: f32,
    local_host: bool,
) -> (AnyWindowHandle, Entity<Workspace>) {
    let (window, workspace) = cx.update(|cx| {
        let mut settings = nocterm_settings::SettingsDocument::default();
        settings.update::<nocterm_ai::AiSettings>(|section| section.enabled = agent);
        settings.update::<nocterm_ui::AppearanceSettings>(|section| section.layout = layout);
        let transport = Arc::new(MockTransport::default());
        let local = if local_host {
            let local = transport.clone();
            nocterm_terminal::LocalSessions::new(
                Arc::new(move |_| local.clone()),
                "/bin/bash".into(),
            )
        } else {
            nocterm_terminal::LocalSessions::disabled("/bin/bash".into())
        };
        let booted = boot_with_ports(
            tempfile::tempdir().unwrap(),
            nocterm_store::SettingsConfig::in_memory(settings),
            (transport, local),
            false,
            None,
            None,
            cx,
        );
        open_workspace(&booted, cx)
    });
    cx.update_window(window, |_, window, cx| {
        window.activate_window();
        window.resize(size(px(width), px(800.)));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    render(cx, window);
    (window, workspace)
}

fn render(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn bounds(items: &[(String, Bounds<Pixels>)], name: &str) -> Bounds<Pixels> {
    items
        .iter()
        .find(|(item, _)| item == name)
        .unwrap_or_else(|| panic!("{name} is not in the footer: {items:?}"))
        .1
}

/// The gaps between the footer's toggles, which sit next to each other.
fn toggle_gaps(items: &[(String, Bounds<Pixels>)]) -> Vec<Pixels> {
    let toggles: Vec<_> = items
        .iter()
        .filter(|(name, _)| {
            name.starts_with("navigation") || name.ends_with("panel") || name == "local-terminal"
        })
        .collect();
    toggles
        .windows(2)
        .filter(|pair| pair[0].0 != "navigation:4" || pair[1].0 != "local-terminal")
        .map(|pair| pair[1].1.left() - pair[0].1.right())
        .collect()
}

fn assert_navigation_presentation(workspace: &Entity<Workspace>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        for (ix, (tooltip, icon)) in NAVIGATION.iter().enumerate() {
            assert_eq!(
                toggle_presentation(workspace.read(cx), Toggle::Navigation(ix), cx),
                Some((SharedString::from(*tooltip), *icon)),
                "navigation toggle {ix}"
            );
        }
        assert_eq!(
            toggle_presentation(workspace.read(cx), Toggle::LocalTerminal, cx),
            Some(("Local Terminal".into(), IconName::SquareTerminal))
        );
    });
}

fn assert_agent_presentation(workspace: &Entity<Workspace>, cx: &mut TestAppContext) {
    let side = |cx: &mut TestAppContext| {
        cx.update(|cx| toggle_presentation(workspace.read(cx), Toggle::SidePanel, cx))
    };
    assert_eq!(
        side(cx),
        Some(("AI Agents".into(), IconName::PanelRight)),
        "resting"
    );
    workspace.update(cx, |workspace, cx| {
        set_side_panel_attention(workspace, true, cx)
    });
    assert_eq!(
        side(cx),
        Some((
            "AI Agents: permission required".into(),
            IconName::ShieldCheck
        )),
        "attention"
    );
    workspace.update(cx, |workspace, cx| {
        set_side_panel_attention(workspace, false, cx)
    });
    assert_eq!(side(cx), Some(("AI Agents".into(), IconName::PanelRight)));
}

/// The width of the monitor button while it has no sample: the icon button.
const MONITOR_ICON_WIDTH: Pixels = px(72.);

const FULL_FOOTER: [&str; 8] = [
    "navigation:0",
    "navigation:1",
    "navigation:2",
    "navigation:3",
    "navigation:4",
    "notice",
    "local-terminal",
    "side-panel",
];

#[gpui_kit::test]
fn floating_layout_at_1280_keeps_its_footer_and_columns(cx: &mut TestAppContext) {
    let (window, workspace) = shell(cx, UiLayout::Floating, true, 1280.);
    cx.update_window(window, |_, window, _| {
        assert_eq!(footer_ids(window), FULL_FOOTER);
        assert_eq!(column_ids(window), ["navigation", "content"]);
        let items = footer_items(window);
        assert_eq!(toggle_gaps(&items), vec![px(4.); 5]);
        assert_eq!(bounds(&items, "navigation:0").left(), px(8.));
        assert_eq!(bounds(&items, "side-panel").right(), px(1272.));
        assert_eq!(
            window.find("workspace-footer").bounds(),
            Bounds::new(point(px(0.), px(768.)), size(px(1280.), px(32.)))
        );
        assert_eq!(
            columns(window)
                .iter()
                .map(|(_, b)| (b.origin.x, b.size.width))
                .collect::<Vec<_>>(),
            vec![(px(5.), px(220.5)), (px(231.5), px(1043.5))]
        );
    })
    .unwrap();
    assert_navigation_presentation(&workspace, cx);
    assert_agent_presentation(&workspace, cx);
}

#[gpui_kit::test]
fn classic_layout_at_1280_keeps_its_footer_and_columns(cx: &mut TestAppContext) {
    let (window, workspace) = shell(cx, UiLayout::Classic, true, 1280.);
    cx.update_window(window, |_, window, _| {
        assert_eq!(footer_ids(window), FULL_FOOTER);
        assert_eq!(column_ids(window), ["navigation", "content"]);
        let items = footer_items(window);
        assert_eq!(toggle_gaps(&items), vec![px(4.); 5]);
        assert_eq!(
            window.find("workspace-footer").bounds(),
            Bounds::new(point(px(0.), px(767.)), size(px(1280.), px(33.)))
        );
        assert_eq!(
            columns(window)
                .iter()
                .map(|(_, b)| (b.origin.x, b.size.width))
                .collect::<Vec<_>>(),
            vec![(px(0.), px(226.5)), (px(226.5), px(1053.5))]
        );
    })
    .unwrap();
    assert_navigation_presentation(&workspace, cx);
    assert_agent_presentation(&workspace, cx);
}

#[gpui_kit::test]
fn narrow_window_with_a_notice_keeps_the_toggles_at_the_edges(cx: &mut TestAppContext) {
    let (window, workspace) = shell(cx, UiLayout::Floating, true, 760.);
    cx.update_window(window, |_, window, cx| {
        nocterm_ui::notice::info(window, cx, "geometry", "Saved", "Nothing to do");
        window.render_frame(cx);
        assert_eq!(footer_ids(window), FULL_FOOTER);
        assert_eq!(column_ids(window), ["navigation", "content"]);
        let items = footer_items(window);
        assert_eq!(toggle_gaps(&items), vec![px(4.); 5]);
        let notice = bounds(&items, "notice");
        assert!(
            notice.size.width > px(0.),
            "the notice is drawn: {notice:?}"
        );
        assert!(notice.left() >= bounds(&items, "navigation:4").right());
        assert!(notice.right() <= bounds(&items, "local-terminal").left());
        assert_eq!(bounds(&items, "side-panel").right(), px(752.));
        assert_eq!(
            window.find("workspace-footer").bounds(),
            Bounds::new(point(px(0.), px(768.)), size(px(760.), px(32.)))
        );
        assert_eq!(
            columns(window)
                .iter()
                .map(|(_, b)| (b.origin.x, b.size.width))
                .collect::<Vec<_>>(),
            vec![(px(5.), px(186.)), (px(197.), px(558.))]
        );
    })
    .unwrap();
    assert_navigation_presentation(&workspace, cx);
    assert_agent_presentation(&workspace, cx);
}

#[gpui_kit::test]
fn hiding_the_sidebar_leaves_the_footer_and_the_content_column(cx: &mut TestAppContext) {
    let (window, workspace) = shell(cx, UiLayout::Floating, true, 1280.);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.toggle_sidebar(window, cx));
        window.render_frame(cx);
        assert_eq!(footer_ids(window), FULL_FOOTER);
        assert_eq!(column_ids(window), ["content"]);
        let content = columns(window)[0].1;
        assert_eq!(content.origin.x, px(5.));
        assert_eq!(content.size.width, px(1270.));
    })
    .unwrap();
    assert_navigation_presentation(&workspace, cx);
    assert_agent_presentation(&workspace, cx);
}

#[gpui_kit::test]
fn disabled_agent_removes_only_its_toggle(cx: &mut TestAppContext) {
    let (window, workspace) = shell(cx, UiLayout::Floating, false, 1280.);
    cx.update_window(window, |_, window, _| {
        assert_eq!(footer_ids(window), &FULL_FOOTER[..7]);
        assert_eq!(column_ids(window), ["navigation", "content"]);
        let items = footer_items(window);
        assert_eq!(toggle_gaps(&items), vec![px(4.); 4]);
        assert_eq!(bounds(&items, "local-terminal").right(), px(1272.));
        assert_eq!(
            window.find("workspace-footer").bounds(),
            Bounds::new(point(px(0.), px(768.)), size(px(1280.), px(32.)))
        );
    })
    .unwrap();
    assert_navigation_presentation(&workspace, cx);
    cx.update(|cx| {
        assert_eq!(
            toggle_presentation(workspace.read(cx), Toggle::SidePanel, cx),
            None
        );
    });
}

#[gpui_kit::test]
fn a_monitor_without_a_sample_sits_between_the_toggles_and_the_notice(cx: &mut TestAppContext) {
    let (window, workspace) = shell_on(cx, UiLayout::Floating, true, 1280., true);
    cx.update_window(window, |_, window, _| {
        let mut full = FULL_FOOTER.to_vec();
        full.insert(5, "monitor");
        assert_eq!(footer_ids(window), full);
        let items = footer_items(window);
        assert_eq!(toggle_gaps(&items), vec![px(4.); 5]);
        let monitor = bounds(&items, "monitor");
        assert_eq!(
            monitor.left() - bounds(&items, "navigation:4").right(),
            px(4.)
        );
        assert_eq!(monitor.size.width, MONITOR_ICON_WIDTH);
        assert!(monitor.right() <= bounds(&items, "notice").left());
        assert_eq!(bounds(&items, "side-panel").right(), px(1272.));
        assert_eq!(column_ids(window), ["navigation", "content"]);
    })
    .unwrap();
    assert_navigation_presentation(&workspace, cx);
    assert_agent_presentation(&workspace, cx);
}
