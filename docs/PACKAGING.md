# Distribution packaging

The Debian and Fedora files are **review drafts, not upload-ready packages**.
Public source builds use an original demo lexicon; see [PUBLIC_SOURCE.md](PUBLIC_SOURCE.md).
Original code is MIT; the combined data license and precise attribution remain
unresolved. Neither a successful local build nor this draft means acceptance
by a distribution. See [release requirements](RELEASE.md).

## Shared installation contract

Use `make build` with the final `PREFIX`/`DATADIR`, then
`python3 scripts/install.py` with identical paths and `DESTDIR` set to the package
root. `make install` combines those steps. Defaults are `/usr/local`, `bin`,
`share` and `share/man`; distro packages use `/usr`. Compilation happens as the
unprivileged builder, not during a privileged installation hook.

The installer owns the binary, data image, IBus component XML, desktop file,
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
retained as reference; a distro's patched registry can resolve its own versions
offline. `dh_shlibdeps` derives ELF library requirements; IBus/Fontconfig and
Chinese font/portal recommendations are explicit.

Before submission, verify actual package names and versions for zbus 6,
eframe 0.33, rfd 0.15, plist, Tokio, encoding_rs and flate2. The current Rust
requirement is 1.99.1, the tested toolchain, not a claim that older compilers
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
version. The local development host has no Rust RPM macros installed, so a
plain `rpmbuild` here cannot validate those macros or resolve BuildRequires.

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
make install PREFIX="$PWD/build/test-prefix" CARGO_BUILD_FLAGS='--locked --offline'
env -u WAYPINYIN_LEXICON build/test-prefix/bin/waypinyin --check-data
```

`DESTDIR` is not a runtime prefix. An image staged under `build/stage-usr/usr`
is not expected to be found at `/usr/share` on the host before installation.
