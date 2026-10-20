//! The jump hosts of the open form: the ordered hop list and the inputs that extend it.
//!
//! Owns the draft list and the add inputs. Route resolution, persistence and
//! connecting belong to `nocterm_connections_model::jump_route` and the open intent.
use super::{ConnectionEditor, FormError, field, watch};
use gpui_kit::{
    AnyElement, Context, Entity, SharedString, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        select::{SearchableVec, Select, SelectState},
    },
    div,
    prelude::*,
};
use nocterm_connections_model::{
    draft::ValidationField,
    jump_route::JumpHostEntry,
    profiles::{Profile, ProfileId, Profiles},
};
use nocterm_session::Target;
use nocterm_ui::form;

type Picker = Entity<SelectState<SearchableVec<SharedString>>>;

pub(super) struct JumpHostsFields {
    hops: Vec<JumpHostEntry>,
    address: Entity<gpui_kit::component::input::InputState>,
    picker: Picker,
    /// The picker's entries, in the order it shows them.
    choices: Vec<(SharedString, ProfileId)>,
    _subscriptions: Vec<Subscription>,
}

fn label(profile: &Profile) -> SharedString {
    format!("{} — {}", profile.name, profile.target).into()
}

impl JumpHostsFields {
    /// What the picker offers: every saved connection except the one being edited.
    /// Entries that read alike are numbered, so a label picks exactly one connection.
    pub(super) fn choices(own: ProfileId, profiles: &Profiles) -> Vec<(SharedString, ProfileId)> {
        let mut choices: Vec<(SharedString, ProfileId)> = Vec::new();
        for saved in profiles.iter().filter(|saved| saved.id != own) {
            let base = label(saved);
            let mut unique = base.clone();
            let mut number = 1;
            while choices.iter().any(|(existing, _)| *existing == unique) {
                number += 1;
                unique = format!("{base} ({number})").into();
            }
            choices.push((unique, saved.id));
        }
        choices
    }

    pub(super) fn new(
        profile: Option<&Profile>,
        choices: Vec<(SharedString, ProfileId)>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let labels: Vec<SharedString> = choices.iter().map(|(label, _)| label.clone()).collect();
        let picker = cx.new(|cx| {
            SelectState::new(SearchableVec::new(labels), None, window, cx).searchable(true)
        });
        let address = field("user@bastion.example[:port]", String::new(), window, cx);
        let subscription = watch(&address, window, cx);
        Self {
            hops: profile.map(|p| p.jump_hosts.clone()).unwrap_or_default(),
            address,
            picker,
            choices,
            _subscriptions: vec![subscription],
        }
    }

    pub(super) fn snapshot(&self) -> Vec<JumpHostEntry> {
        self.hops.clone()
    }

    pub(super) fn address(&self) -> &Entity<gpui_kit::component::input::InputState> {
        &self.address
    }

    /// Appends the typed `user@host[:port]`, using `default_user` when it has none.
    pub(super) fn add_address(
        &mut self,
        default_user: Option<&str>,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Result<(), FormError> {
        let text = self.address.read(cx).value().to_string();
        let target = Target::parse(&text, default_user).map_err(|error| {
            FormError::new(
                ValidationField::JumpHosts,
                format!("Jump host address: {error}."),
            )
        })?;
        self.hops.push(JumpHostEntry::Address(target));
        self.address
            .update(cx, |input, cx| input.set_value("", window, cx));
        Ok(())
    }

    /// Appends the connection chosen in the picker; false when none is chosen.
    pub(super) fn add_selected(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> bool {
        let Some(chosen) = self.picker.read(cx).selected_value().cloned() else {
            return false;
        };
        let Some((_, id)) = self.choices.iter().find(|(label, _)| *label == chosen) else {
            return false;
        };
        self.hops.push(JumpHostEntry::Profile(*id));
        self.picker
            .update(cx, |picker, cx| picker.set_selected_index(None, window, cx));
        true
    }

    pub(super) fn remove(&mut self, index: usize) {
        if index < self.hops.len() {
            self.hops.remove(index);
        }
    }

    pub(super) fn move_up(&mut self, index: usize) {
        if index > 0 && index < self.hops.len() {
            self.hops.swap(index - 1, index);
        }
    }

    pub(super) fn move_down(&mut self, index: usize) {
        if index + 1 < self.hops.len() {
            self.hops.swap(index, index + 1);
        }
    }
}

impl JumpHostsFields {
    /// How each hop reads in the list: an address as typed, a saved connection
    /// by name, or a note that the connection was deleted.
    pub(super) fn labels(&self, profiles: &Profiles) -> Vec<SharedString> {
        self.hops
            .iter()
            .map(|hop| match hop {
                JumpHostEntry::Address(target) => target.to_string().into(),
                JumpHostEntry::Profile(id) => profiles
                    .get(*id)
                    .map_or_else(|| "Deleted connection".into(), label),
            })
            .collect()
    }

    fn render_hop(
        &self,
        index: usize,
        description: SharedString,
        cx: &mut Context<ConnectionEditor>,
    ) -> AnyElement {
        let last = index + 1 == self.hops.len();
        let tool = |id: &'static str, text: &'static str, disabled: bool| {
            Button::new((id, index))
                .small()
                .ghost()
                .label(text)
                .disabled(disabled)
        };
        let description = format!("{}. {description}", index + 1);
        h_flex()
            .id(("jump-host", index))
            .test_support()
            .role(gpui_kit::Role::ListItem)
            .aria_label(description.clone())
            .w_full()
            .py_2()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(description),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(tool("jump-host-up", "Up", index == 0).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.jump_hosts.move_up(index);
                            cx.notify();
                        },
                    )))
                    .child(tool("jump-host-down", "Down", last).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.jump_hosts.move_down(index);
                            cx.notify();
                        },
                    )))
                    .child(
                        tool("jump-host-remove", "Remove", false).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.jump_hosts.remove(index);
                                this.error = None;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn page(
        &self,
        labels: Vec<SharedString>,
        cx: &mut Context<ConnectionEditor>,
    ) -> Vec<AnyElement> {
        let hops: Vec<AnyElement> = if self.hops.is_empty() {
            vec![form::note("No jump hosts: the connection is direct.", cx)]
        } else {
            labels
                .into_iter()
                .enumerate()
                .map(|(index, description)| self.render_hop(index, description, cx))
                .collect()
        };
        let add_address = Button::new("jump-host-add-address")
            .small()
            .label("Add")
            .on_click(cx.listener(|this, _, window, cx| this.add_jump_address(window, cx)));
        let add_profile = Button::new("jump-host-add-profile")
            .small()
            .label("Add")
            .on_click(cx.listener(|this, _, window, cx| {
                if this.jump_hosts.add_selected(window, cx) {
                    this.error = None;
                }
                cx.notify();
            }));
        vec![
            form::section("Route", hops, cx),
            form::section(
                "Add a jump host",
                [
                    form::row(
                        "Address",
                        "Reached with automatic sign-in. Press Enter to add.",
                        h_flex()
                            .gap_2()
                            .child(
                                div().w(gpui_kit::rems(17.)).child(
                                    gpui_kit::component::input::Input::new(&self.address)
                                        .id("editor-field-Jump host address")
                                        .small(),
                                ),
                            )
                            .child(add_address),
                        cx,
                    ),
                    form::row(
                        "Saved connection",
                        "Uses that connection's address, sign-in and saved password.",
                        h_flex()
                            .gap_2()
                            .child(
                                div().w(gpui_kit::rems(17.)).child(
                                    Select::new(&self.picker).id("jump-host-picker").small(),
                                ),
                            )
                            .child(add_profile),
                        cx,
                    ),
                ],
                cx,
            ),
            form::note(
                "Each jump host is checked and signed in to, in order, before the next one \
                 is reached. A proxy applies to the first one only.",
                cx,
            ),
        ]
    }
}
