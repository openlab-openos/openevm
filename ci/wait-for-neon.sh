#!/bin/bash
set -euo pipefail

: ${EVM_LOADER:=$(solana address -k evm_loader-keypair.json)}
: ${SOLANA_URL:?is not set}

./wait-for-solana.sh "$@"

if [ $# -eq 0 ]; then
  if neon-cli --url $SOLANA_URL --commitment confirmed --evm_loader $EVM_LOADER --loglevel off init-environment > /dev/null; then
    echo "Executed neon-cli init-environment successfully"
    exit 0
  fi
else
  WAIT_TIME=$1
  echo "Waiting $WAIT_TIME seconds for NeonEVM to be available at $SOLANA_URL:$EVM_LOADER"
  for i in $(seq 1 $WAIT_TIME); do
    echo "Try neon-cli init-environment count=$i"
    if neon-cli --url $SOLANA_URL --commitment confirmed --evm_loader $EVM_LOADER --loglevel off init-environment > /dev/null; then
      echo "Executed neon-cli init-environment successfully after count=$i"
      exit 0
    fi
    sleep 1
  done
fi

echo "unable to connect to NeonEVM at $SOLANA_URL:$EVM_LOADER"
exit 1
