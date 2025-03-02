ARG SOLANA_IMAGE
# Install BPF SDK
FROM solanalabs/rust:1.73.0 AS builder
RUN cargo install rustfilt
WORKDIR /opt
ARG SOLANA_BPF_VERSION
RUN sh -c "$(curl -sSfL https://release.solana.com/v1.17.34/install)" && \
    /root/.local/share/solana/install/active_release/bin/sdk/sbf/scripts/install.sh
ENV PATH=${PATH}:/root/.local/share/solana/install/active_release/bin


# Build evm_loader
FROM builder AS evm-loader-builder
COPY .git /opt/neon-evm/.git
COPY evm_loader /opt/neon-evm/evm_loader
WORKDIR /opt/neon-evm/evm_loader
ARG REVISION=1.14.0
ENV NEON_REVISION=${REVISION}
RUN cargo fmt --check && \
    cargo clippy --release && \
    cargo build --release && \
    cargo test --release && \
    cargo build-bpf --manifest-path program/Cargo.toml --features mainnet && cp target/deploy/evm_loader.so target/deploy/evm_loader-mainnet.so 


# Add neon_test_invoke_program to the genesis
FROM neonlabsorg/neon_test_programs:latest AS neon_test_programs

# Define solana-image that contains utility
FROM builder AS base


COPY --from=evm-loader-builder /opt/neon-evm/evm_loader/target/deploy/evm_loader*.so /opt/
COPY --from=evm-loader-builder /opt/neon-evm/evm_loader/target/deploy/evm_loader-dump.txt /opt/
COPY --from=evm-loader-builder /opt/neon-evm/evm_loader/target/release/neon-cli /opt/
COPY --from=evm-loader-builder /opt/neon-evm/evm_loader/target/release/neon-api /opt/
COPY --from=evm-loader-builder /opt/neon-evm/evm_loader/target/release/neon-rpc /opt/
COPY --from=evm-loader-builder /opt/neon-evm/evm_loader/target/release/libneon_lib.so /opt/libs/current/

COPY ci/wait-for-solana.sh \
    ci/wait-for-neon.sh \
    ci/solana-run-neon.sh \
    ci/deploy-evm.sh \
    ci/deploy-multi-tokens.sh \
    ci/create-test-accounts.sh \
    ci/evm_loader-keypair.json \
    /opt/

COPY solidity/ /opt/solidity
COPY ci/operator-keypairs/ /opt/operator-keypairs
COPY ci/operator-keypairs/id.json /root/.config/solana/id.json
COPY ci/operator-keypairs/id2.json /root/.config/solana/id2.json
COPY ci/keys/ /opt/keys

ENV PATH=${PATH}:/opt

ENTRYPOINT [ "/opt/solana-run-neon.sh" ]
