//! The name fields of the known servers.

use std::collections::HashMap;

use gpui_kit::{App, Context, Entity, Subscription, Window};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::{HubId, HubsSettings};
use nocterm_ui::SettingsTextField;

/// The name field of one server: the shared text field of the settings, which keeps what the
/// user typed until its save is settled, saves a distinct text once and shows the stored name
/// again when the settings change elsewhere.
struct NameField {
    field: SettingsTextField,
    _observed: Subscription,
}

/// The name fields of the known servers, one each.
pub(in crate::servers) struct NameFields {
    hubs: Entity<Hubs>,
    fields: HashMap<HubId, NameField>,
}

impl NameFields {
    pub(in crate::servers) fn new(hubs: Entity<Hubs>) -> Self {
        Self {
            hubs,
            fields: HashMap::new(),
        }
    }

    /// The field of `hub`, once [`NameFields::sync`] made it.
    pub(in crate::servers) fn get(&self, hub: &HubId) -> Option<SettingsTextField> {
        self.fields.get(hub).map(|name| name.field.clone())
    }

    /// Saves what is typed in every field that has unsaved text.
    pub(in crate::servers) fn flush(&self, window: &mut Window, cx: &mut App) {
        for name in self.fields.values() {
            name.field.flush(window, cx);
        }
    }

    /// Gives each known server a name field and drops the fields of forgotten ones. A field
    /// that exists keeps what the user is typing. The context owner is redrawn when a field changes.
    pub(in crate::servers) fn sync<T: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<T>,
    ) {
        let known: Vec<HubId> = self
            .hubs
            .read(cx)
            .list()
            .into_iter()
            .map(|s| s.id)
            .collect();
        self.fields.retain(|hub, _| known.contains(hub));
        for hub in known {
            if self.fields.contains_key(&hub) {
                continue;
            }
            let Some(address) = self
                .hubs
                .read(cx)
                .record(&hub)
                .map(|record| record.address.to_string())
            else {
                continue;
            };
            let (reading, writing) = (hub.clone(), hub.clone());
            let field = SettingsTextField::new(
                self.hubs.read(cx).settings(),
                format!("server-name-input-{address}"),
                move |document| {
                    document
                        .get::<HubsSettings>()
                        .find(&reading)
                        .map(|record| record.label.clone())
                        .unwrap_or_default()
                },
                move |document, text| {
                    document.update::<HubsSettings>(|settings| {
                        settings.set_label(&writing, text);
                    });
                    Ok(())
                },
                window,
                cx,
            );
            let page = cx.weak_entity();
            let observed = field.observe(cx, move |cx| {
                let _ = page.update(cx, |_, cx| cx.notify());
            });
            self.fields.insert(
                hub,
                NameField {
                    field,
                    _observed: observed,
                },
            );
        }
    }
}
