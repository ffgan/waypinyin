PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
MANDIR ?= $(DATADIR)/man
DESTDIR ?=
CARGO ?= cargo
CARGO_BUILD_FLAGS ?= --locked
export PREFIX BINDIR DATADIR MANDIR DESTDIR CARGO CARGO_BUILD_FLAGS

.PHONY: all build install check clean dist
all: build
build:
	python3 scripts/build.py
install:
	python3 scripts/install.py
check:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --locked --all-targets -- -D warnings
	$(CARGO) test --locked
	python3 scripts/check_sources.py
	python3 -m unittest discover -s tests -p 'test_*.py'
clean:
	$(CARGO) clean
	rm -rf build
dist:
	python3 scripts/dist.py
