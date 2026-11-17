# Distribution packaging

The Debian, Fedora and Arch files are **review drafts, not upload-ready packages**.
Public source builds use an original demo lexicon; see [PUBLIC_SOURCE.md](PUBLIC_SOURCE.md).
Original code is MIT; the combined data license and precise attribution remain
unresolved. Neither a successful local build nor this draft means acceptance
by a distribution. See [release requirements](RELEASE.md).

## Shared installation contract

Use `make build` with the final `PREFIX`/`DATADIR`, then
`python3 scripts/install.py` with identical paths and `DESTDIR` set to the package
root. `make install` only stages existing build artifacts; it never invokes Cargo.
Run `make build` explicitly before installation. Defaults are `/usr/local`, `bin`,
`share` and `share/man`; distro packages use `/usr`. Compilation happens as the
unprivileged builder, not during a privileged installation hook.

Files are atomically replaced so a running engine can retain its executable
and mapped image until restart. The installer owns the binary, data image, IBus component XML, desktop file,
scalable icon, manual, documentation and license inventory. Package removal
must not touch user settings, learning records or imported dictionaries. No
post-install service restart or automatic input-source change is required.

The image is generated locally from shipped text. Its record representation
is native little endian; initial package architectures are x86_64 and aarch64.
Generate an image on the matching target architecture and keep it in the same
architecture-specific binary package. Do not mark it `Architecture: all` or
`BuildArch: noarch`. Cross compilation is rejected by the build script.

## Debian / Ubuntu

Copy `packaging/debian` to a top-level `debian/` in a separate packaging checkout.
Set a real maintainer identity before any upload. Build with `sbuild` against a
chosen Debian unstable or Ubuntu development suite; run `lintian` on source
and binary artifacts. Install, upgrade and remove in a disposable test system,
checking that user files survive. GNOME Wayland registration still requires a
real desktop acceptance test.

The draft configures `/usr/share/cargo/registry` as Cargo's source and disables
network access. Its direct crate Build-Depends must be resolved in the target
suite, along with all enabled transitive features. The upstream lockfile is
backed up during configuration and restored by clean; a fresh lockfile is
resolved offline from the suite registry, including its patched crate checksums. `dh_shlibdeps` derives ELF library requirements; IBus/Fontconfig and
Chinese font/portal recommendations are explicit.

Before submission, verify actual package names and versions for zbus 5.19,
eframe 0.33, rfd 0.15, plist, Tokio, encoding_rs and flate2. The current Rust
requirement is 1.89.0, a tested minimum for the locked native graph, not a claim that older compilers
cannot work. Lower it only after the full native graph and tests pass there.
Do not bypass missing packages with a network fetch. A new crate package,
feature adjustment or separately tested compatibility patch is required.

Policy references:
- https://www.debian.org/doc/debian-policy/ch-source.html
- https://wiki.debian.org/Teams/RustPackaging/Policy
- https://ubuntu.com/project/docs/maintainers/niche-package-maintenance/rustc/rust-packaging-faq/

## Fedora

Use `packaging/fedora/waypinyin.spec` with `mock` and the selected Fedora release.
The draft uses `%cargo_prep` and `%cargo_generate_buildrequires` with system
registry sources. Verify macro options against the target `rust-packaging`
version. The native host lacks the Cargo RPM macros. A disposable Fedora 45 container
has verified that `%cargo_generate_buildrequires -n` expands to the expected
no-default-features invocation; an actual package build still requires the
complete target registry and successful dependency resolution.

Replace `LicenseRef-waypinyin-unresolved` with the reviewed expression covering
actual shipped data, code, static crates and assets. Add a real URL/Source0 once
the upstream release location exists. Generate and review Cargo license/notice
metadata for the **actual enabled features**; the inventory helper is only a
starting point. BuildRequires must be available in Fedora, including crates
requested by dynamic BuildRequires. Run `mock`, `rpmlint`, install/upgrade/remove
tests and a GNOME Wayland session test before treating the spec as ready.

Default packaging disables bundled egui fonts. Fontconfig must supply usable
Latin and Chinese glyphs. Builds with the default bundled-fonts feature need
their font notices audited and shipped too. There is no bundled Chinese font.

Policy references:
- https://docs.fedoraproject.org/en-US/packaging-guidelines/Rust/
- https://docs.fedoraproject.org/en-US/packaging-guidelines/LicensingGuidelines/
- https://docs.fedoraproject.org/en-US/reproducible-builds/

## Local checks

```sh
make check
make build PREFIX=/usr CARGO_BUILD_FLAGS='--locked --offline'
DESTDIR="$PWD/build/stage-usr" PREFIX=/usr python3 scripts/install.py
python3 scripts/verify_install.py "$PWD/build/stage-usr" --prefix /usr
python3 scripts/dependency_inventory.py
python3 scripts/check_reproducible.py
```

For a runnable relocation test, build using an absolute temporary prefix:

```sh
make build PREFIX="$PWD/build/test-prefix" CARGO_BUILD_FLAGS='--locked --offline'
make install PREFIX="$PWD/build/test-prefix"
env -u WAYPINYIN_LEXICON build/test-prefix/bin/waypinyin --check-data
```

`DESTDIR` is not a runtime prefix. An image staged under `build/stage-usr/usr`
is not expected to be found at `/usr/share` on the host before installation.

## Arch Linux

`packaging/arch/PKGBUILD` is a review template for the cleared demo-source
edition. It has been tested with a local cleared archive in an isolated Arch
container; its release URL and checksum are still placeholders. Set the actual release
URL and SHA-256 before running `makepkg`; the placeholder deliberately fails
source verification. Fetch locked sources in `prepare()`, then build and test
with `--frozen` and system fonts. Review linked dependency notices, run namcap
and validate in a clean chroot before submission. Do not use this template with
unresolved historical data or claim the demo is a production dictionary.

## Other distributions and downstream overrides

The same staged installer can be used by openSUSE, Gentoo, Alpine and other
packagers. No acceptance or build support for those distributions is asserted.
The current native image requires a little-endian target matching the build
host. Validate the toolchain, native libraries, portal integration and tests
on every additional architecture or libc before offering packages.

`CARGO`, `CARGO_BUILD_FLAGS`, `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET` and Rust
compiler flags are inherited from the package build environment. Use
`--offline --no-default-features` with a distribution registry, or `--frozen
--no-default-features` with previously fetched locked sources. The installer
requires no Cargo, network, compiler or service activation.

Override `BINDIR`, `DATADIR` and `MANDIR` with absolute paths as needed. Build
with the final `PREFIX` and `DATADIR`; pass the same values at installation.
`DESTDIR` only stages files and is never embedded in desktop or IBus metadata.
The verification helper accepts `--bindir`, `--datadir` and `--mandir` to check
such layouts. The desktop launcher uses the final absolute binary path.

## Provenance and dependency notices

Package preparation rejects unresolved data with `scripts/check_sources.py
--release`, even when runtime tests are skipped. Use a cleared public export;
never build a distributable binary from the historical corpus until its remaining
source records have been resolved with evidence.

`make build` records Cargo's actual compiler-artifact package IDs and collects
notices for those packages, including host/build dependencies. Notices missing
from crate archives are supplied by pinned, hash-checked source-revision
supplements under LICENSES. The installation verifier checks every inventory
notice is present in the staged package. An inventory is not a substitute for
choosing the linked-license expression required by the target distribution.

To exercise engine discovery and real wire messages without changing the active
desktop input source, install PyGObject, the IBus typelib, ibus-daemon and D-Bus,
then run:

```sh
python3 scripts/check_ibus.py build/waypinyin build/lexicon.bin
```

This uses a private session bus, a private IBus socket and temporary user data.
It validates preedit, lookup-table and commit signals through an actual daemon.
Visual glyph quality, portal interaction and native Wayland focus behavior still
require a desktop acceptance run.
