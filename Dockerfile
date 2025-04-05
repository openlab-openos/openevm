# Install BPF SDK
FROM ubuntu:22.04
# Build evm_loader
COPY .git /opt/neon-evm/.git
COPY evm_loader /opt/neon-evm/evm_loader
WORKDIR /opt/neon-evm/evm_loader
ENV NEON_REVISION=1.14.0
COPY evm_loader/target/deploy/evm_loader.so evm_loader/target/deploy/evm_loader-mainnet.so 

COPY evm_loader/target/deploy/evm_loader*.so /opt/
COPY evm_loader/target/release/neon-cli /opt/
COPY evm_loader/target/release/neon-api /opt/
COPY evm_loader/target/release/neon-rpc /opt/
COPY evm_loader/target/release/libneon_lib.so /opt/libs/current/

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
