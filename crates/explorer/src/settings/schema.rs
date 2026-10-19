//! Stable public schema identity for the established settings format.
use super::*;
impl JsonSchema for ExplorerSettings {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ExplorerSettings".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_files::settings::ExplorerSettings".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field(
            "indexing",
            &default.indexing,
            "Counting the files and the size of the folder being shown.",
        );
        schema.field("open", &default.open, "The programs that open files.");
        schema.finish("The Explorer in the sidebar.")
    }
}

impl JsonSchema for IndexingSettings {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "IndexingSettings".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_files::settings::IndexingSettings".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field(
            "local",
            &default.local,
            "Count the files and the size of local folders.",
        );
        schema.field(
            "remote",
            &default.remote,
            "Count the files and the size of remote folders, over SFTP.",
        );
        schema.field("skip_home", &default.skip_home, "Do not count a home directory itself: it is usually huge. Its\nsubfolders are still counted.");
        schema.field(
            "skip_root",
            &default.skip_root,
            "Do not count the root of a file system (`/`, `C:\\`).",
        );
        schema.field("excluded", &default.excluded, "Folders with these names are never entered while counting, wherever\nthey are. The folder being shown is always counted.");
        schema.field(
            "max_local_entries",
            &default.max_local_entries,
            "Stop counting a local folder after this many entries.",
        );
        schema.range(
            "max_local_entries",
            INDEXING_ENTRIES_RANGE.start(),
            INDEXING_ENTRIES_RANGE.end(),
        );
        schema.field("max_remote_entries", &default.max_remote_entries, "Stop counting a remote folder after this many entries. Every folder\ncosts a round trip to the server.");
        schema.range(
            "max_remote_entries",
            INDEXING_ENTRIES_RANGE.start(),
            INDEXING_ENTRIES_RANGE.end(),
        );
        schema.finish("Counting the files and the size of the folder being shown.")
    }
}

impl JsonSchema for Opener {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Opener".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_files::settings::Opener".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field(
            "program",
            &default.program,
            "The executable, such as `nvim` or `code`. Empty uses the default.",
        );
        schema.field("args", &default.args, "Arguments, passed one by one without shell parsing. `{file}` stands for\nthe file; without it the file is added last.");
        schema.finish("A program and its arguments.")
    }
}

impl JsonSchema for OpenSettings {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "OpenSettings".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_files::settings::OpenSettings".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field("local", &default.local, "Opens files on this computer. Empty uses the system's default\napplication for the file.");
        schema.field("remote", &default.remote, "Opens files on a server, in a new terminal tab on that server: an\neditor such as `nano`, `vim` or `nvim`.");
        schema.field("rules", &default.rules, "Programs for particular file types. The first rule naming a file's\nextension with a program for that side wins; with none, the defaults\nabove are used.");
        schema.finish("The programs that open files.")
    }
}

impl JsonSchema for OpenRule {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "OpenRule".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_files::settings::OpenRule".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field("extensions", &default.extensions, "Extensions without the dot, such as `md` or `tar.gz`, or whole file\nnames such as `Dockerfile`. Case is ignored.");
        schema.field(
            "local",
            &default.local,
            "Opens matching files on this computer.",
        );
        schema.field(
            "remote",
            &default.remote,
            "Opens matching files on a server.",
        );
        schema.finish("Programs for files with particular extensions.")
    }
}
