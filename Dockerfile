FROM rust:latest AS BUILDER

RUN rustup set profile minimal && rustup target add x86_64-unknown-linux-musl
RUN apt update && apt install -y musl-tools musl-dev --no-install-recommends
RUN update-ca-certificates

ENV USER=simpleproxy
ENV UID=10001

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

RUN mkdir -p /usr/src/
WORKDIR /usr/src/
COPY src/ /usr/src/src
COPY Cargo.toml /usr/src/
COPY Cargo.lock /usr/src/

RUN cargo build --target x86_64-unknown-linux-musl --release

FROM alpine
WORKDIR /simpleproxy

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group
COPY --from=builder /usr/src/target/x86_64-unknown-linux-musl/release/simpleproxy ./

RUN chown -R simpleproxy:simpleproxy /simpleproxy && chmod -R 774 /simpleproxy

USER simpleproxy:simpleproxy
ENTRYPOINT ["/simpleproxy/simpleproxy", "--config", "/simpleproxy/config.toml"]