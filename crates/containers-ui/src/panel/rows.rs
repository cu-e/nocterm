//! The panel's tree: section and project headers, container and image rows,
//! and the buttons and menu each offers.

use gpui_kit::{
    AnyElement, ClickEvent, ClipboardItem, Context, Hsla, SharedString,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        menu::{ContextMenuExt as _, PopupMenuItem},
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_containers::{Container, Group, Image, State};
use nocterm_ui::IconName;

use super::{ContainersPanel, Fold, Kind, Subject};
use crate::ops::{self, Op};

/// Shared by every row, so hovering a row reveals only its own buttons.
const ROW_GROUP: &str = "container-row";
/// Same for project headers.
const PROJECT_GROUP: &str = "container-project";

impl ContainersPanel {
    pub(super) fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.model.read(cx).snapshot().clone();
        let mut tree = v_flex().gap_px().pb_1();

        let containers = !self.folded.contains(&Fold::Containers);
        tree = tree.child(self.render_section(
            Fold::Containers,
            "Containers",
            snapshot.containers.len(),
            cx,
        ));
        if containers {
            if snapshot.containers.is_empty() {
                tree = tree.child(Self::render_empty("No containers.", cx));
            }
            for group in snapshot.groups() {
                tree = match group.project {
                    Some(project) => tree.child(self.render_project(project, &group, cx)),
                    None => tree.children(
                        group
                            .containers
                            .iter()
                            .map(|container| self.render_container(container, cx)),
                    ),
                };
            }
        }

        let images = !self.folded.contains(&Fold::Images);
        tree = tree.child(self.render_section(Fold::Images, "Images", snapshot.images.len(), cx));
        if images {
            if snapshot.images.is_empty() {
                tree = tree.child(Self::render_empty("No images.", cx));
            }
            let mut sorted: Vec<&Image> = snapshot.images.iter().collect();
            sorted.sort_by_key(|image| (image.repository.is_none(), image.name()));
            tree = tree.children(sorted.into_iter().map(|image| self.render_image(image, cx)));
        }
        tree
    }

    fn render_section(
        &self,
        fold: Fold,
        title: &'static str,
        count: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let folded = self.folded.contains(&fold);
        h_flex()
            .id(SharedString::from(format!("containers-section-{title}")))
            .test_support()
            .gap_1()
            .mx_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(Icon::new(chevron(folded)).xsmall())
            .child(div().flex_1().child(title.to_uppercase()))
            .child(count.to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(fold.clone(), cx)))
    }

    fn render_project(
        &self,
        project: &str,
        group: &Group<'_>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fold = Fold::Project(project.to_owned());
        let folded = self.folded.contains(&fold);
        let subject = Subject {
            kind: Kind::Project,
            name: project.to_owned(),
            ids: group.ids(),
        };
        let key = format!("project-{project}");
        let header = h_flex()
            .id(SharedString::from(key.clone()))
            .test_support()
            .group(PROJECT_GROUP)
            .gap_1p5()
            .mx_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(
                Icon::new(chevron(folded))
                    .xsmall()
                    .text_color(theme.muted_foreground),
            )
            .child(Icon::new(IconName::Boxes).small().text_color(state_color(
                group.running() > 0,
                false,
                cx,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(project.to_owned()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{}/{}", group.running(), group.containers.len())),
            )
            .child(self.render_buttons(&key, PROJECT_GROUP, &ops::for_project(group), &subject, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(fold.clone(), cx)))
            .context_menu(self.menu(ops::for_project(group), subject, None, cx));

        v_flex().gap_px().child(header).when(!folded, |column| {
            column.child(
                v_flex().pl_3().gap_px().children(
                    group
                        .containers
                        .iter()
                        .map(|container| self.render_container(container, cx)),
                ),
            )
        })
    }

    fn render_container(&self, container: &Container, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let ops = ops::for_container(container);
        let subject = Subject {
            kind: Kind::Container,
            name: container.name.clone(),
            ids: vec![container.id.clone()],
        };
        let color = state_color(
            container.state == State::Running,
            matches!(container.state, State::Paused | State::Restarting),
            cx,
        );
        let key = format!("container-{}", container.id);
        let detail = match container.status.is_empty() {
            true => container.image.clone(),
            false => format!("{} · {}", container.image, container.status),
        };
        let logs = subject.clone();
        h_flex()
            .id(SharedString::from(key.clone()))
            .test_support()
            .group(ROW_GROUP)
            .gap_2()
            .mx_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(Icon::new(IconName::Container).small().text_color(color))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(container.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    ),
            )
            .child(self.render_buttons(&key, ROW_GROUP, &ops, &subject, cx))
            // The row is about its output, as in a log viewer.
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.run(Op::Logs, logs.clone(), window, cx)
            }))
            .context_menu(self.menu(ops, subject, Some(container.id.clone()), cx))
            .into_any_element()
    }

    fn render_image(&self, image: &Image, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let subject = Subject {
            kind: Kind::Image,
            name: image.name(),
            ids: vec![image.id.clone()],
        };
        let key = format!("image-{}", image.id);
        let detail = [image.size.as_str(), image.created.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        h_flex()
            .id(SharedString::from(key.clone()))
            .test_support()
            .group(ROW_GROUP)
            .gap_2()
            .mx_1()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .hover(|row| row.bg(theme.sidebar_accent))
            .child(
                Icon::new(IconName::Image)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(subject.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    ),
            )
            .child(self.render_buttons(&key, ROW_GROUP, &ops::FOR_IMAGE, &subject, cx))
            .context_menu(self.menu(ops::FOR_IMAGE.to_vec(), subject, Some(image.id.clone()), cx))
            .into_any_element()
    }

    fn render_empty(text: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .mx_1()
            .px_2()
            .py_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }

    /// The buttons a row shows while `hover_group` is hovered.
    fn render_buttons(
        &self,
        key: &str,
        hover_group: &'static str,
        ops: &[Op],
        subject: &Subject,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .invisible()
            .group_hover(hover_group, |buttons| buttons.visible())
            .children(ops.iter().copied().filter(|op| op.inline()).map(|op| {
                let subject = subject.clone();
                Button::new(SharedString::from(format!("{key}-{}", op.key())))
                    .ghost()
                    .xsmall()
                    .icon(op.icon())
                    .tooltip(op.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.run(op, subject.clone(), window, cx);
                    }))
            }))
    }

    /// A row's menu: every op, with what destroys set apart, and copying
    /// the id when the row has one.
    fn menu(
        &self,
        ops: Vec<Op>,
        subject: Subject,
        id: Option<String>,
        cx: &mut Context<Self>,
    ) -> impl Fn(
        gpui_kit::component::menu::PopupMenu,
        &mut gpui_kit::Window,
        &mut Context<gpui_kit::component::menu::PopupMenu>,
    ) -> gpui_kit::component::menu::PopupMenu
    + 'static {
        let panel = cx.entity().downgrade();
        move |mut menu, _, _| {
            let item = |op: Op| {
                let panel = panel.clone();
                let subject = subject.clone();
                PopupMenuItem::new(op.label()).on_click(move |_, window, cx| {
                    let _ = panel.update(cx, |this, cx| this.run(op, subject.clone(), window, cx));
                })
            };
            let mut sections = vec![
                ops.iter()
                    .copied()
                    .filter(|op| !op.destructive())
                    .map(item)
                    .collect::<Vec<_>>(),
            ];
            if let Some(id) = id.clone() {
                sections.push(vec![PopupMenuItem::new("Copy ID").on_click(
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(id.clone())),
                )]);
            }
            sections.push(
                ops.iter()
                    .copied()
                    .filter(|op| op.destructive())
                    .map(item)
                    .collect(),
            );
            for (ix, section) in sections.into_iter().filter(|s| !s.is_empty()).enumerate() {
                if ix > 0 {
                    menu = menu.separator();
                }
                for entry in section {
                    menu = menu.item(entry);
                }
            }
            menu
        }
    }
}

fn chevron(folded: bool) -> IconName {
    if folded {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    }
}

/// Green for what runs, amber for what is held, grey for the rest.
fn state_color(running: bool, held: bool, cx: &gpui_kit::App) -> Hsla {
    let theme = cx.theme();
    if running {
        theme.success
    } else if held {
        theme.warning
    } else {
        theme.muted_foreground
    }
}
