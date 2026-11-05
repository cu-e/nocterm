//! One immutable managed layout, rendered into child shells or native launch variables.
use nocterm_session::ShellLaunch;

const ROOT: &str = ".local/share/nocterm/toolchains";
const PATHS: [&str; 5] = [
    "bin",
    "rust/cargo/bin",
    "bun/current/bin",
    "node/current/bin",
    "java/current/bin",
];
const EXPORTS: [(&str, &str, &str); 4] = [
    ("CARGO_HOME", "rust/cargo", "rust/cargo/bin/rustup"),
    ("RUSTUP_HOME", "rust/rustup", "rust/cargo/bin/rustup"),
    ("BUN_INSTALL", "bun/current", "bun/current/bin/bun"),
    ("JAVA_HOME", "java/current", "java/current/bin/java"),
];

pub(crate) fn path_export(base: &str) -> String {
    let paths = PATHS.map(|path| format!("${base}/{path}")).join(":");
    format!("export PATH=\"{paths}:$PATH\"\n")
}
pub(crate) fn shell() -> String {
    let mut script = format!(
        "if [ -n \"${{HOME:-}}\" ]; then\nnocterm_toolchains=\"$HOME/{ROOT}\"\nif [ -d \"$nocterm_toolchains\" ] && [ ! -L \"$nocterm_toolchains\" ]; then\n"
    );
    script.push_str(&path_export("nocterm_toolchains"));
    for (name, path, probe) in EXPORTS {
        script.push_str(&format!("if [ -x \"$nocterm_toolchains/{probe}\" ]; then export {name}=\"$nocterm_toolchains/{path}\"; fi\n"));
    }
    script.push_str("fi\nunset nocterm_toolchains\nfi\n");
    script
}
pub(crate) fn local(launch: &mut ShellLaunch) {
    let home = launch
        .env
        .get("HOME")
        .cloned()
        .or_else(|| std::env::var("HOME").ok());
    let Some(home) = home else { return };
    let root = std::path::Path::new(&home).join(ROOT);
    if !std::fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.is_dir()) {
        return;
    }
    let inherited = launch
        .env
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let mut paths: Vec<_> = PATHS.iter().map(|path| root.join(path)).collect();
    paths.extend(std::env::split_paths(&inherited));
    if let Ok(path) = std::env::join_paths(paths) {
        launch
            .env
            .insert("PATH".into(), path.to_string_lossy().into_owned());
    }
    for (name, path, probe) in EXPORTS {
        if root.join(probe).is_file() {
            launch
                .env
                .insert(name.into(), root.join(path).to_string_lossy().into_owned());
        }
    }
}
