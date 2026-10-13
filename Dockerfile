# amdtop — btop-style terminal monitor for AMD Strix Halo.
#
#   docker build -t amdtop .                                      runtime image (default)
#   docker build --target builder -t amdtop-dev .                 toolchain + sources
#   docker build --target export --output type=local,dest=dist .  just the binary
#
# Build args:
#   RUST_VERSION  Rust image tag, default "1" (latest stable)
#   DEBIAN        Debian release shared by builder and runtime, default "bookworm"

ARG RUST_VERSION=1
ARG DEBIAN=bookworm

# --------------------------------------------------------------- builder ----
FROM rust:${RUST_VERSION}-slim-${DEBIAN} AS builder

# gcc + libc6-dev give cargo a linker. Every dependency (ratatui, crossterm,
# libc) is pure Rust, so no pkg-config/openssl headers are needed.
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc libc6-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cargo.toml references README.md and LICENSE, so both must be present before
# any cargo command runs.
COPY Cargo.toml Cargo.lock README.md LICENSE ./

# Warm the dependency cache behind a throwaway main.rs, so that editing src/
# later only recompiles the amdtop crate itself.
RUN mkdir -p src \
 && printf 'fn main() {}\n' > src/main.rs \
 && cargo build --release --locked \
 && rm -rf src

COPY src ./src
# src/ is newer than the throwaway build, so this rebuilds the binary only.
RUN cargo build --release --locked

# Toolchain image for building and testing a mounted checkout:
#   docker run --rm -v "$PWD":/app -v amdtop-target:/app/target -w /app \
#     amdtop-dev cargo test --locked
FROM builder AS dev
CMD ["cargo", "test", "--locked"]

# ---------------------------------------------------------------- export ----
# Writes the release binary to ./dist/amdtop without keeping an image.
FROM scratch AS export
COPY --from=builder /app/target/release/amdtop /amdtop
COPY --from=builder /app/LICENSE /LICENSE

# --------------------------------------------------------------- runtime ----
FROM debian:${DEBIAN}-slim AS runtime

LABEL org.opencontainers.image.title="amdtop" \
      org.opencontainers.image.description="btop-style terminal monitor for AMD Strix Halo: CPU, GPU, VRAM/GTT/TTM and per-process GPU memory" \
      org.opencontainers.image.licenses="LicenseRef-MIT-Commons-Clause"

COPY --from=builder /app/target/release/amdtop /usr/local/bin/amdtop
# MIT requires the notice to travel with the binary, and the Commons Clause
# requires its own notice to travel with it too.
COPY --from=builder /app/LICENSE /usr/local/share/doc/amdtop/LICENSE

# amdtop reads /proc and /sys, which inside a container are the container's own.
# For live host monitoring:
#   docker run --rm -it --pid=host -v /sys:/sys:ro amdtop
# Add --privileged for the TTM tab, which needs the host's debugfs:
#   docker run --rm -it --pid=host --privileged -v /sys:/sys:ro amdtop
# Without AMD hardware, --demo renders a simulated Strix Halo:
#   docker run --rm -it amdtop --demo
ENTRYPOINT ["amdtop"]
CMD ["--help"]
