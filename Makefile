# Waffle — common tasks. Run `make help` for the list.
.PHONY: site help app app-debug run test test-rust test-swift selftest lint fmt probe corpus corpus-large icon clean

SELFTEST_FILES := styled.xlsx formulas.xlsx rich.xlsx comma.csv legacy.xls

help:            ## Show this help
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  make %-14s %s\n", $$1, $$2}'

app:             ## Build build/Waffle.app (release)
	@scripts/bundle.sh release

app-debug:       ## Build build/debug/Waffle.app with test hooks
	@scripts/bundle.sh debug

run: app         ## Build and launch the app
	@open build/Waffle.app

test: test-rust test-swift  ## Run all tests

test-rust:       ## Rust unit + round-trip tests
	cargo test --workspace --release

test-swift:      ## Swift bridge tests
	cargo build --release -p waffle-ffi
	cd macos && swift run -c release BridgeTests

selftest: app-debug  ## Drive the real app through editing/undo/save on sample files
	@set -e; tmp=$$(mktemp -d); for f in $(SELFTEST_FILES); do \
	  case $$f in *.csv) ext=csv;; *) ext=xlsx;; esac; \
	  out=$$(WAFFLE_SELFTEST=$$tmp/out.$$ext build/debug/Waffle.app/Contents/MacOS/Waffle "$$PWD/testdata/files/$$f" 2>&1); \
	  fails=$$(echo "$$out" | grep -c '^FAIL' || true); oks=$$(echo "$$out" | grep -c '^ok' || true); \
	  echo "$$f: $$oks ok, $$fails failed"; echo "$$out" | grep '^FAIL' || true; [ "$$fails" = 0 ]; done

lint:            ## rustfmt check + clippy (warnings are errors)
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --release -- -D warnings

fmt:             ## Format Rust code
	cargo fmt --all

probe:           ## Load/save timing: make probe FILE=path [OUT=path]
	cargo run --release -q -p waffle-probe -- $(FILE) $(OUT)

corpus:          ## Regenerate small test files (needs python deps, see testdata/README.md)
	python3 tools/corpus/generate.py --skip-large && python3 tools/corpus/validate.py

corpus-large:    ## Also generate the 10k×1k and 200k-row files (~100 s)
	python3 tools/corpus/generate.py && python3 tools/corpus/validate.py

icon:            ## Regenerate the logo, wordmark lockups and app icon
	@swift tools/icon/make_icon.swift tools/icon >/dev/null && \
	  iconutil -c icns tools/icon/AppIcon.iconset -o macos/Resources/AppIcon.icns && \
	  rm -rf tools/icon/AppIcon.iconset && \
	  cp tools/icon/logo.png tools/icon/lockup-light.png tools/icon/lockup-dark.png site/assets/ && \
	  echo "✓ tools/icon/{logo,lockup-light,lockup-dark}.png (also in site/assets), macos/Resources/AppIcon.icns"

clean:           ## Remove build outputs
	rm -rf build macos/.build target

site:            ## Build the website into build/site (then open build/site/index.html)
	@python3 tools/site/build.py
