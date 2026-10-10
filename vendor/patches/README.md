# Vendored source patches

Each fork is reconstructed from its checksum-pinned **published crates.io archive**,
the exact omitted-file list and ordered Git patches in [sources.json](sources.json):

```
published archive - listed omissions + ordered patches = vendor/<package>
```

Run the check with Python 3.11 or newer and Git; no Rust compilation is needed:

```sh
python3 scripts/test-vendor-patches.py
python3 scripts/check-vendor-patches.py
```

The checker downloads bounded HTTPS inputs to `target/vendor-patches/archives`,
authenticates every cached archive again, safely extracts to a temporary directory,
applies patches with `git apply --check --index` and `git apply --index`, and
compares the complete source set, bytes and executable bits. It excludes only
ignored Cargo `target/` directories. Untracked source additions also fail. There
is no three-way repair or whitespace correction. Corrupt cache entries fail;
remove the reported corrupt artifact before retrying a download.

For an offline check, provide all named `.crate` files in one directory:

```sh
python3 scripts/check-vendor-patches.py --archives-dir /path/to/archives
```

Missing offline artifacts fail without making a network request. File checksums
can be independently checked against crates.io's version metadata. The checker
also validates revision evidence inside each authenticated published archive.

## Source provenance

The archive is the exact base, including normalized Cargo manifests and publisher
packaging. It is not interchangeable with a checkout of the recorded upstream
revision. `cargo-vcs` evidence means the archive's `.cargo_vcs_info.json` records
that revision and repository path. `publisher-zed-rev` means the publisher's
`package.metadata.gpui-pre.zed-rev` asserts that revision; those GPUI archives have
no `.cargo_vcs_info.json`. The manifest records the upstream repository, path and
revision for investigation, without asserting that a raw git checkout reproduces
publisher transformations.

The Alacritty archive has 184 explicitly omitted packaging/fixture files; tid-rs
has five. The manifest lists every path, without ignore globs. Omitting a file
which is absent from the archive fails. Each historical fork starts with one
consolidated binary patch, because its original local edits were not independently
sequenced. These patches include the fork's Nocterm notes, tests, locks and added
binary fixtures. They preserve every current vendored source byte.
The consolidated patches encode all changed files as Git binary deltas, including
text files. This preserves licensed text bytes and patch context without
introducing whitespace exceptions for generated patch data.
Each file ends with a comment after Git's required final binary-record blank
delimiter. Preserve that delimiter: a truncated stream can omit its final delta;
the full byte comparison detects the mismatch.

The six Apache license files match their published archives. The tid-rs publisher
omitted its MIT license; the patch adds the license from the exact upstream
revision, with its URL and SHA-256 recorded in `additional_sources`. Font fixtures
retain their individual OFL licenses and provenance manifests in the corresponding
forks; a crate's surrounding Apache license does not replace font licenses.

## Updating a fork

Add a separately ordered `.patch` file under that package's directory and append
its path to `sources.json`. Generate binary patches with
`git diff --binary --full-index` against the last replayed state, preserving file
modes. In the disposable reconstruction repository, `.git/info/attributes` with
`* -diff` makes Git encode text files as binary patches too; it is not committed
to a fork. Preserve Git's full output and append
`# End of generated Nocterm binary patch.` after its final blank delimiter.
When changing the published base, update its version, archive URL,
registry checksum, revision evidence and exact omission list, then regenerate
patches against that authenticated base. Review omissions and license provenance.

Run the checker and fork-specific behavioral tests before submitting. Source
replay proves bytes, not native platform behavior. Windows/macOS tests remain
necessary for native patches. Keep vendor changes in separate commits from
`crates/` and `src/` changes, and review notice inputs under
[Packaging](../../docs/PACKAGING.md#license-notices). Patch files themselves do not
replace license notices or upstream modification notices.
