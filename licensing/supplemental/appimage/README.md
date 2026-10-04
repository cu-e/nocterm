# AppImage launcher licenses and corresponding source

The AppImage includes the separate AppImage type-2 launcher release `20251108`,
source commit `dd6cebedcbddde9c82f89b011e8e1d40b6e43868`:
<https://github.com/AppImage/type2-runtime/tree/dd6cebedcbddde9c82f89b011e8e1d40b6e43868>.
The launcher is MIT licensed and statically links musl, libfuse, squashfuse,
zstd, zlib and mimalloc. Original license texts, including libfuse's LGPL-2.1
terms, are retained beside this README in the repository and included in the
installed `THIRD_PARTY_NOTICES.txt`. Source URLs and snapshot checksums are in
`licensing/sources.json`. Nocterm's application executable runs separately from
the launcher and is licensed under PolyForm Perimeter.

The source versions fixed by the launcher's dependency build script are
libfuse 3.15.0 and squashfuse 0.5.2. The other license snapshots come from musl
1.2.5, zstd 1.5.6, zlib 1.3.1 and mimalloc 2.1.7, contemporary with Alpine 3.21.
Upstream's Docker image and Alpine package inputs float, so these license source
versions do not establish the exact package revisions in the prebuilt launcher.
The upstream build does not guarantee byte-for-byte reproducibility.

## Included sources and relinking

Inside the AppImage, `usr/share/doc/nocterm/appimage-runtime-sources/` contains
this README, `appimage-runtime.json`, the full launcher source archive, libfuse
3.15.0 source archive and squashfuse 0.5.2 source archive. The manifest pins the
binary and archive SHA-256 values; packaging verifies all downloaded and cached
files. libfuse's full source includes GPL-2.0 development tools as well as the
LGPL-2.1 library; both license texts are preserved. Nocterm does not install those
development tools as runtime executables.

To rebuild the launcher with a modified libfuse, extract the archives into a
working directory:

```sh
tar xf type2-runtime-dd6cebedcbddde9c82f89b011e8e1d40b6e43868.tar.gz
tar xf fuse-3.15.0.tar.xz
tar xf squashfuse-0.5.2.tar.gz
cd type2-runtime-dd6cebedcbddde9c82f89b011e8e1d40b6e43868
```

The launcher archive includes the complete source, Makefile, linker script,
Docker build scripts and `patches/libfuse/mount.c.diff`. To use your local
library sources in the Docker build:

```sh
mkdir dependency-sources
cp -a ../fuse-3.15.0 ../squashfuse-0.5.2 dependency-sources/
```

Make your libfuse changes in `dependency-sources/fuse-3.15.0`. Add
`COPY dependency-sources/ /sources/` to `scripts/docker/Dockerfile` before its
`RUN bash scripts/common/install-dependencies.sh` line. In that dependency
script, replace the three libfuse download/checksum/extraction commands with
`cp -a /sources/fuse-3.15.0 .`, and the three squashfuse commands with
`cp -a /sources/squashfuse-0.5.2 .`. Retain the libfuse `patch -p1` command, which
applies the included patch exactly once, and the existing static-library build
commands. Resolve any patch conflicts with your changes before building.
Retain the other Alpine dependencies. `src/runtime/Makefile` links `-lfuse3` and
the other static libraries; the following build relinks against your library.

An unmodified upstream build can be started using:

```sh
printf '%s\n' dd6cebedcbddde9c82f89b011e8e1d40b6e43868 > src/runtime/version
ARCH=x86_64 bash scripts/docker/build-with-docker.sh
```

See the archive's `BUILD.md` for environment requirements. Building requires
Docker, network access and the dependencies installed by the included scripts.
The output launcher is `runtime-x86_64` in the current directory. Repackage an
extracted AppImage directory using your launcher:

```sh
appimagetool --runtime-file ./runtime-x86_64 --no-appstream squashfs-root Modified-Nocterm.AppImage
```

The original AppImage can be extracted with `--appimage-extract`. This replaces
the launcher and allows modification of the LGPL library without changing or
recompiling the Nocterm application. The distribution provides the launcher work
and library sources under their original terms; this README imposes no additional
restriction on modifying or debugging the LGPL component.
