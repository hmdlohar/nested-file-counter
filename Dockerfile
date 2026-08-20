FROM rust:1-slim

WORKDIR /work

# Cache-friendly: copy manifests first is handled by compose/makefile mounts;
# this image is used via `docker run -v $PWD:/work` so no COPY needed for dev.
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*

CMD ["cargo", "build"]
