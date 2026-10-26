//! A developer session saves nothing of its plugins: it loads no installed plugin, writes no
//! records, data or package file, and a second session starts empty. (Its settings stay in
//! memory too; see `startup`.)
use std::{fs, path::Path, sync::Arc};

use gpui_kit::TestAppContext;
use nocterm_core::clock::{WallClock, WallTime};
use nocterm_plugin_api::{ContributionContext, Event, Kind, LocalId, Zone};
use nocterm_plugin_host::{Call, Input};
use nocterm_plugin_model::{PluginId, SettingValue, SettingValues};
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugins_ui::{InstallRequest, UserPlugins as _};
use nocterm_store::ShutdownCoordinator;
use tempfile::TempDir;

use super::{UserShell, manager::HostPlugins};
use crate::shell_setup::{UserPluginMode, plugin_catalog};

struct Noon;
impl WallClock for Noon {
    fn now(&self) -> WallTime {
        WallTime::default()
    }
}

/// A code package that stores one entry whenever an event reaches it. Calls whose first byte
/// is 0 are renders and get a view of one text node; the others get the effect.
const MANIFEST: &str = r#"
api = "0.1"
publisher = "example"
id = "stamp"
name = "Example Stamp"
runtime = "wasm"
capabilities = ["storage.set"]

[reasons]
"storage.set" = "To remember a stamp"

[[contribution]]
local = "face"
kind = "widget"
title = "Stamp"
icon = "puzzle"
zone = "right"

[settings]
version = 1

[[settings.group]]
title = "Stamp"

[[settings.group.field]]
key = "label"
label = "Label"
kind = "text"
default = "Hi"
max_len = 12
"#;

fn module() -> Vec<u8> {
    let section: String = MANIFEST
        .bytes()
        .map(|byte| format!("\\{byte:02x}"))
        .collect();
    wat::parse_str(format!(
        r#"(module
            (@custom "nocterm.manifest" "{section}")
            (memory (export "memory") 2)
            (data (i32.const 1024) "\00\01\00\00\02\02\68\69\00\00\00")
            (data (i32.const 2048) "\01\01\01\04\73\61\76\65\03\01\6b\01\76\00")
            (func (export "nocterm_abi_version") (result i32) (i32.const 1))
            (func (export "nocterm_alloc") (param i32) (result i32) (i32.const 65536))
            (func (export "nocterm_call") (param i32 i32) (result i64)
                (if (result i64) (i32.load8_u (i32.const 65536))
                    (then (i64.const 8796093022222))
                    (else (i64.const 4398046511115)))))"#
    ))
    .expect("the test module is valid WAT")
}

/// Everything one session leaves for a test to look at.
struct Session {
    shell: UserShell,
    plugins: HostPlugins,
    state: TempDir,
    file: String,
    _work: TempDir,
}

fn session(mode: impl FnOnce(&Path) -> UserPluginMode, cx: &mut TestAppContext) -> Session {
    let state = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let file = work.path().join("stamp.wasm");
    fs::write(&file, module()).unwrap();
    let catalog = Arc::new(plugin_catalog().expect("the application catalog"));
    let paths = nocterm_core::Paths::rooted_at(state.path());
    let mode = mode(&file);
    let settings = super::gate::allowing(cx);
    let dev = match &mode {
        UserPluginMode::Dev(file) => Some(file.clone()),
        UserPluginMode::Installed | UserPluginMode::SafeMode => None,
    };
    let shell = cx.update(|cx| {
        UserShell::install(
            &catalog,
            &paths,
            &ShutdownCoordinator::default(),
            Arc::new(Noon),
            mode,
            &settings,
            cx,
        )
    });
    Session {
        plugins: HostPlugins::new(
            shell.host().expect("the shell loads user plugins").clone(),
            dev,
        ),
        shell,
        state,
        file: file.to_str().unwrap().to_owned(),
        _work: work,
    }
}

fn dev(file: &Path) -> UserPluginMode {
    UserPluginMode::Dev(file.to_owned())
}

fn installed(_: &Path) -> UserPluginMode {
    UserPluginMode::Installed
}

fn plugin() -> PluginId {
    PluginId::user("example", "stamp").unwrap()
}

/// Consents to the package, changes a setting and presses the widget so that it stores an entry,
/// then lets the debounced writes of an installation land. The same steps run in every mode.
fn use_the_plugin(session: &Session, cx: &mut TestAppContext) {
    let candidate = cx
        .update(|cx| session.plugins.inspect(&session.file, cx))
        .expect("the package is acceptable");
    cx.update(|cx| {
        session.plugins.install(
            InstallRequest {
                location: session.file.clone(),
                hash: candidate.hash,
                replaces: None,
                accepted: Vec::new(),
                keep_data: false,
            },
            cx,
        )
    })
    .expect("the consent installs the plugin");

    let mut values = SettingValues::default();
    values.insert("label", SettingValue::Text("Hey".into()));
    session
        .shell
        .host()
        .expect("the shell loads user plugins")
        .update(cx, |host, cx| {
            host.replace_values(&plugin(), &values, cx)
                .expect("the values are stored");
        });
    assert_eq!(
        session
            .shell
            .host()
            .expect("the shell loads user plugins")
            .read_with(cx, |host, _| host.values(&plugin())),
        Some(values),
        "the session keeps what the user set"
    );

    cx.add_empty_window();
    let window = cx.windows()[0].window_id();
    let call = Call {
        window,
        plugin: plugin(),
        local: LocalId::parse("face").unwrap(),
        context: ContributionContext {
            kind: Kind::Widget,
            zone: Zone::Right,
        },
        in_focus: true,
    };
    session
        .shell
        .host()
        .expect("the shell loads user plugins")
        .update(cx, |host, cx| {
            host.handle(
                &call,
                Input::Pointer,
                Event::Activated { key: "go".into() },
                &PluginProviders::new(),
                cx,
            )
            .expect("the plugin takes the event");
        });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    cx.run_until_parked();
}

fn listed(session: &Session, cx: &mut TestAppContext) -> usize {
    cx.update(|cx| session.plugins.rows(cx)).len()
}

#[gpui_kit::test]
fn a_developer_session_writes_no_file_and_the_next_one_starts_empty(cx: &mut TestAppContext) {
    let first = session(dev, cx);
    assert_eq!(listed(&first, cx), 0, "no installed plugin is loaded");
    use_the_plugin(&first, cx);
    assert_eq!(listed(&first, cx), 1);

    assert!(
        !first.state.path().join("state/plugins").exists(),
        "a developer session creates no plugin directory"
    );
    assert_eq!(
        fs::read_dir(first.state.path()).unwrap().count(),
        0,
        "nothing at all is written under the application's directories"
    );

    let second = session(dev, cx);
    assert_eq!(listed(&second, cx), 0, "a second session starts empty");
}

#[gpui_kit::test]
fn an_installation_does_write_its_records_which_makes_the_control_meaningful(
    cx: &mut TestAppContext,
) {
    let installation = session(installed, cx);
    use_the_plugin(&installation, cx);

    let plugins = installation.state.path().join("state/plugins");
    assert!(
        plugins.join("records.json").is_file(),
        "the same steps write the records when the ports are the installed ones"
    );
    assert!(
        plugins
            .join("data")
            .join(format!("{}.json", plugin()))
            .is_file(),
        "and the plugin's data, which a dev session's `storage.set` must not reach"
    );
}

#[gpui_kit::test]
fn the_dev_session_lists_the_file_it_loads(cx: &mut TestAppContext) {
    let first = session(dev, cx);
    assert_eq!(
        cx.update(|cx| first.plugins.dev_location(cx)),
        Some(first.file.clone())
    );
    let candidate = cx
        .update(|cx| first.plugins.inspect(&first.file, cx))
        .unwrap();
    assert!(candidate.dev, "the consent of a session is a DEV consent");

    let installation = session(installed, cx);
    assert_eq!(cx.update(|cx| installation.plugins.dev_location(cx)), None);
    let candidate = cx
        .update(|cx| installation.plugins.inspect(&installation.file, cx))
        .unwrap();
    assert!(!candidate.dev);
}
