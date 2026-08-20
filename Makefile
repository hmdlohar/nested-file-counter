IMAGE ?= hnfc-dev
WORKDIR ?= /work
CACHE_VOL ?= hnfc-cargo-registry

# Use host UID/GID so built binaries are owned correctly on Linux
UID ?= $(shell id -u)
GID ?= $(shell id -g)

DOCKER_RUN = docker run --rm -w $(WORKDIR) -v $(PWD):$(WORKDIR) -v $(CACHE_VOL):/usr/local/cargo/registry

.PHONY: build image check test lint fmt run dist scan

image:
	docker build -t $(IMAGE) .

build: image
	$(DOCKER_RUN) $(IMAGE) cargo build

build-release: image
	$(DOCKER_RUN) $(IMAGE) cargo build --release
	@echo "Binary: target/release/hnfc"

check: image
	$(DOCKER_RUN) $(IMAGE) cargo check

test: image
	$(DOCKER_RUN) $(IMAGE) cargo test

lint: image
	$(DOCKER_RUN) $(IMAGE) cargo clippy -- -D warnings

fmt: image
	$(DOCKER_RUN) $(IMAGE) cargo fmt -- --check
fmt-fix: image
	$(DOCKER_RUN) $(IMAGE) cargo fmt

# Run TUI against a path (default: current project). Mounts host root ro for scanning arbitrary paths.
# Usage: make run ARGS="/ --hidden"  or  make run ARGS="--help"
run: image
	docker run --rm -it -w $(WORKDIR) -v $(PWD):$(WORKDIR) -v $(CACHE_VOL):/usr/local/cargo/registry -v /:/host:ro $(IMAGE) cargo run -- $(ARGS)

# One-shot example without TUI
scan: image
	docker run --rm -w $(WORKDIR) -v $(PWD):$(WORKDIR) -v $(CACHE_VOL):/usr/local/cargo/registry -v /:/host:ro $(IMAGE) cargo run -- --no-tui --top 20 $(ARGS)

# Cross builds via cargo-zigbuild (pure Docker, no host toolchains)
dist:
	mkdir -p dist
	docker run --rm -w $(WORKDIR) -v $(PWD):$(WORKDIR) -v $(CACHE_VOL):/usr/local/cargo/registry ghcr.io/rust-cross/cargo-zigbuild:latest cargo zigbuild --release --target x86_64-unknown-linux-musl
	docker run --rm -w $(WORKDIR) -v $(PWD):$(WORKDIR) -v $(CACHE_VOL):/usr/local/cargo/registry ghcr.io/rust-cross/cargo-zigbuild:latest cargo zigbuild --release --target aarch64-unknown-linux-musl || true
	cp target/x86_64-unknown-linux-musl/release/hnfc dist/hnfc-linux-amd64 || true
	cp target/aarch64-unknown-linux-musl/release/hnfc dist/hnfc-linux-arm64 || true
	ls -lh dist/

clean:
	$(DOCKER_RUN) $(IMAGE) cargo clean
	rm -rf dist
