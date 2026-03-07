.PHONY: fmt lint test build check

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

test:
	cargo test

build:
	cargo build --release

check: fmt lint test
