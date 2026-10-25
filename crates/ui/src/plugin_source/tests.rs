use std::{cell::Cell, rc::Rc};

use gpui_kit::TestAppContext;
use nocterm_plugin_model::{PluginCatalog, PluginSet};

use super::*;

fn system() -> PluginSet {
    let groups = [nocterm_plugin_model::GroupSpec {
        id: nocterm_plugin_model::GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }];
    PluginCatalog::new(&groups, &[], &[]).unwrap().into()
}

#[gpui_kit::test]
fn a_static_source_returns_the_same_set_and_never_notifies(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let source = StaticPluginSource::new(system());
        let first = source.current(cx);
        let second = source.current(cx);
        assert!(Arc::ptr_eq(&first, &second));
        drop(source.observe(Box::new(|_| panic!("a static source never publishes")), cx));
    });
}

#[gpui_kit::test]
fn a_published_set_reaches_every_observer_once(cx: &mut TestAppContext) {
    let seen = Rc::new(Cell::new(0));
    let (handle, before, _subscription) = cx.update(|cx| {
        let handle = PluginSetHandle::new(system(), cx);
        let count = seen.clone();
        let subscription = handle.observe(Box::new(move |_| count.set(count.get() + 1)), cx);
        let before = handle.current(cx);
        (handle, before, subscription)
    });

    cx.update(|cx| {
        handle.replace(before.with_users(Vec::new(), 1).unwrap(), cx);
        handle.replace(before.with_users(Vec::new(), 1).unwrap(), cx);
    });

    cx.update(|cx| {
        assert_eq!(handle.current(cx).revision(), 1);
        assert!(!Arc::ptr_eq(&before, &handle.current(cx)));
    });
    assert_eq!(seen.get(), 1, "the repeated revision was not published");
}
