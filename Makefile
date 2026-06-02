.PHONY: build build-dev fmt fmt-check lint test pack-release

build:
	wasm-pack build --target web --scope lemons_dev

build-dev:
	wasm-pack build --dev --target web --scope lemons_dev

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

lint:
	cargo clippy

test:
	wasm-pack test --node

pack-release: build
	npm pack ./pkg
