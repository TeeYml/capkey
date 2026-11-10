TARGET := riscv64imac-unknown-none-elf

.PHONY: build test lint fmt

build:
	cargo build --release --target $(TARGET) -p capkey-lock

test: build
	cargo test -p tests -- --nocapture --test-threads=1

lint:
	cargo clippy --release --target $(TARGET) -p capkey-lock -- -D warnings
	cargo clippy -p tests --tests -- -D warnings

fmt:
	cargo fmt --all
