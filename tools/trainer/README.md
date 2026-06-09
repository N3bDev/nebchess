# nebchess-trainer (offline, plan-10)

Standalone bullet-based NNUE trainer. NOT part of the engine build (own `[workspace]`).
Requires `CUDA_PATH=/usr/local/cuda` and the RTX 5080 (sm_120).

## Pipeline
1. Build bullet-utils: `cd /home/witt/bullet && cargo build --release --package bullet-utils`
2. Convert+shuffle shards: `tools/trainer/prepare-data.sh <shard-dir> <out.bin> [mem_mb]`
3. Train: `cd tools/trainer && CUDA_PATH=/usr/local/cuda ./target/release/nebchess-trainer --data <out.bin> --id <name> --superbatches <N> [--bps <B>]`
4. Net: `tools/trainer/checkpoints/<name>-<N>/quantised.bin` (raw i16, column-major, LE, padded to /64; this is the net contract for plan-11). For HIDDEN=768 the file is 1184320 bytes.

Arch: `(768 -> 768)x2 -> 1` SCReLU, QA=255, QB=64, SCALE=400, WDL=0.2 (eval-dominant). Tunables: HIDDEN, --superbatches, --bps, the wdl/lr schedulers.
