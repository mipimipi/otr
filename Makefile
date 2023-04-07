PROG=otr

SHELL=/usr/bin/bash

# Set project VERSION to last tag name. If no tag exists, set it to v0.0.0
$(eval TAGS=$(shell git rev-list --tags))
ifdef TAGS
	VERSION=$(shell git describe --tags --abbrev=0)
else
	VERSION=v0.0.0	
endif

# Build executable 
all:
	cargo build --release

.PHONY: all install lint release

lint:
	reuse lint

install:
	@install -Dm755 target/release/$(PROG) $(DESTDIR)/usr/bin/$(PROG)

# Call make release RELEASE=vX.Y.Z
# (1) Adjust version in Cargo.toml and PKGBUILD to RELEASE, commit and push
#     changes
# (2) Create an annotated tag with name RELEASE
release:
	@if [ -z $(RELEASE) ]; then \
		echo "no new release submitted"; \
		exit 1; \
	fi
	@VER_NEW=$(RELEASE); \
	VER_NEW=$${VER_NEW#v}; \
	VER_OLD=`sed -n "s/^version *= \"*\(.*\)\"/\1/p" ./Cargo.toml`; \
	if ! [ $$((`vercmp $${VER_OLD} $${VER_NEW}`)) -lt 0 ]; then \
		echo "new version is not greater than old version"; \
		exit 1; \
	fi; \
	sed -i -e "s/^version.*/version = \"$${VER_NEW#v}\"/" ./Cargo.toml;
	@git commit -a -s -m "release $(RELEASE)"
	@git push
	@git tag -a $(RELEASE) -m "release $(RELEASE)"
	@git push origin $(RELEASE)
