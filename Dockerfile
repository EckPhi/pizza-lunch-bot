FROM rust:1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN install -d -o 10001 -g 10001 -m 0700 /data
COPY --from=build /build/target/release/pizza-lunch-bot /usr/local/bin/pizza-lunch-bot
ENV LUNCH_STATE_FILE=/data/lunch-state.json
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/pizza-lunch-bot"]
