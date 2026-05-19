.PHONY: all build test check lint complexity clean bench bench-real bench-embed

all: build test lint complexity

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy -- -D warnings

complexity:
	arborist --sort cognitive --threshold 15 --exceeds-only src/ || echo "NOTE: cognitive complexity exceeds threshold in some functions"

check: lint test complexity

bench:
	cargo run --bin bench

bench-embed:
	cargo run --bin bench-embed

bench-real: build
	REPO=/tmp/just bash benches/real-repo.sh

clean:
	cargo clean
	rm -rf .sift/
