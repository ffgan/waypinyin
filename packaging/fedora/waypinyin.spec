# REVIEW DRAFT: not uploadable until provenance, license expression and crate
# availability are resolved. See docs/PACKAGING.md and docs/RELEASE.md.
Name:           waypinyin
Version:        0.1.0
Release:        1%{?dist}
Summary:        Offline Pinyin input method for IBus
License:        LicenseRef-waypinyin-unresolved
URL:            https://github.com/ffgan/waypinyin
Source0:        %{name}-%{version}.tar.gz
ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo >= 1.89.0
BuildRequires:  rust >= 1.89.0
BuildRequires:  rust-packaging
BuildRequires:  make
BuildRequires:  python3 >= 3.11
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  desktop-file-utils
Requires:       ibus
Requires:       fontconfig
Recommends:     google-noto-sans-cjk-fonts
Recommends:     xdg-desktop-portal

%description
Offline Chinese Pinyin input through IBus, primarily for GNOME Wayland,
with local learning, fuzzy Pinyin and dictionary management.

%prep
%autosetup
python3 scripts/check_sources.py --release
# Use Fedora's system Cargo registry; never vendor or fetch in the sandbox.
%cargo_prep

%generate_buildrequires
%cargo_generate_buildrequires -n

%build
# Preserve Fedora's Cargo/Rust flags while using the shared upstream installer.
export CARGO_NET_OFFLINE=true
export CARGO_BUILD_FLAGS='--offline --no-default-features'
%make_build build PREFIX=%{_prefix} BINDIR=%{_bindir} DATADIR=%{_datadir}

%install
PREFIX=%{_prefix} BINDIR=%{_bindir} DATADIR=%{_datadir} MANDIR=%{_mandir} DESTDIR=%{buildroot} python3 scripts/install.py
# Keep docs and license files owned by the standard RPM directives below.
rm -rf %{buildroot}%{_datadir}/doc/waypinyin %{buildroot}%{_datadir}/licenses/waypinyin

%check
cargo test --offline --no-default-features
desktop-file-validate %{buildroot}%{_datadir}/applications/waypinyin-settings.desktop

%files
%license LICENSE data/NOTICE data/sources.json LICENSES/*.txt build/dependency-licenses.json build/dependency-notices
%doc README.md README.en.md CHANGELOG.md docs/PACKAGING.md docs/RELEASE.md docs/VALIDATION.md
%{_bindir}/waypinyin
%{_datadir}/waypinyin/
%{_datadir}/ibus/component/waypinyin.xml
%{_datadir}/applications/waypinyin-settings.desktop
%{_datadir}/icons/hicolor/scalable/apps/waypinyin.svg
%{_mandir}/man1/waypinyin.1*

%changelog
* Sat Nov 14 2026 emt - 0.1.0-1
- Local packaging draft; not released
