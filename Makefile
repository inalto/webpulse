# WebPulse — developer tasks (no cPanel needed). See docs/DEVELOPMENT.md.

PHP     ?= php
PORT    ?= 8099
VERSION := $(shell cat VERSION)
DEVCFG  := tests/tmp/dev/config.json
GEOIP_FIXTURE := tests/fixtures/geoip/dbip-country-lite.mmdb

PHP_FILES = $(shell find bin lib tests cpanel whm hooks installer scripts -type f \( -name '*.php' -o -name '*.cgi' -o -name 'webpulse' \) 2>/dev/null | grep -v '^tests/tmp/')
JS_FILES  = $(shell find ui -type f -name '*.js' 2>/dev/null)

.PHONY: test lint fixtures serve serve-admin dist geoip-fixture clean

test: lint
	$(PHP) tests/run.php

lint:
	@fail=0; for f in $(PHP_FILES); do \
		if head -c 200 "$$f" | grep -q '<?php\|#!/usr/bin/env php\|bin/php'; then \
			out=$$($(PHP) -l "$$f" 2>&1) || { echo "$$out"; fail=1; }; \
		fi; \
	done; \
	if command -v node >/dev/null 2>&1; then \
		for f in $(JS_FILES); do node --check "$$f" || fail=1; done; \
	fi; \
	if [ $$fail -ne 0 ]; then echo "lint failed"; exit 1; fi; \
	echo "lint ok"

fixtures:
	$(PHP) tests/support/devenv.php dev

$(DEVCFG):
	$(PHP) tests/support/devenv.php dev

# serve / serve-admin print http://127.0.0.1:$(PORT)/?token=… — open that URL (the token is required)
serve: $(DEVCFG)
	WEBPULSE_CONFIG=$(DEVCFG) $(PHP) bin/webpulse serve --port $(PORT) --user alice

serve-admin: $(DEVCFG)
	WEBPULSE_CONFIG=$(DEVCFG) $(PHP) bin/webpulse serve --port $(PORT) --admin

geoip-fixture:
	$(PHP) tests/support/geoip_fixture.php

dist:
	@if [ -x scripts/build_dist.sh ]; then scripts/build_dist.sh; else \
		mkdir -p dist && tar --exclude='./tests/tmp' --exclude='./dist' --exclude='./.git' \
			--transform 's,^\.,webpulse-$(VERSION),' -czf dist/webpulse-$(VERSION).tar.gz . && \
		echo "dist/webpulse-$(VERSION).tar.gz"; fi

clean:
	rm -rf tests/tmp dist
