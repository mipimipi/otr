# Makefile is only used to create a new release. Usage:
# 	make RELEASE=vX.Y.Z

# use bash
SHELL=/usr/bin/bash

# (1) adjust version in cargo.toml and PKGBUILD, commit and push changes
# (2) create an annotated tag with name RELEASE
all:
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
	sed -i -e "s/^version.*/version = \"$${VER_NEW#v}\"/" ./Cargo.toml; \
	sed -i -e "s/pkgver=.*/pkgver=$${VER_NEW#v}/" ./pkg/PKGBUILD
	@git commit -a -s -m "release $(RELEASE)"
	@git push
	@git tag -a $(RELEASE) -m "release $(RELEASE)"
	@git push origin $(RELEASE)
