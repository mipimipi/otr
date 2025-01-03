# SPDX-FileCopyrightText: 2022-2024 Michael Picht <mipi@fsfe.org>
#
# SPDX-License-Identifier: GPL-3.0-or-later

PROG=otr

# Set variables whose values are specific to the operation system. Currently only
# Linux and macOS (Darwin) are supported.
# TARGETDIR is required for the install recipe
OS=$(shell uname)
ifeq ($(OS), Linux)
TARGETDIR=/usr/bin
else ifeq ($(OS), Darwin)
TARGETDIR=/usr/local/bin
else
$(error $(PROG) is only running on Linux or macOS)
endif

# Build executable 
all:
	cargo build --release $(BUILD_FLAGS)

.PHONY: all install lint release

lint:
	reuse lint

install:
	mkdir -p $(DESTDIR)$(TARGETDIR)
	cp target/release/$(PROG) $(DESTDIR)$(TARGETDIR)/.

# Call make release RELEASE=X.Y.Z
# (1) Adjust version in Cargo.toml and Cargo.lock (via "cargo update") to
#     RELEASE, commit and push
#     changes
# (2) Create an annotated tag with name RELEASE
release:
	@if [ -z $(RELEASE) ]; then \
		echo "no new release submitted"; \
		exit 1; \
	fi
	@VER_NEW=$(RELEASE); \
	VER_OLD=`sed -n "s/^version *= \"*\(.*\)\"/\1/p" ./Cargo.toml`; \
	if ! [ $$((`vercmp $${VER_OLD} $${VER_NEW}`)) -lt 0 ]; then \
		echo "new version is not greater than old version"; \
		exit 1; \
	fi; \
	sed -i -e "s/^version.*/version = \"$${VER_NEW}\"/" ./Cargo.toml; \
	cargo update
	@git commit -a -s -m "release $(RELEASE)"
	@git push
	@git tag -a $(RELEASE) -m "release $(RELEASE)"
	@git push origin $(RELEASE)
