# collect_note

Pool fee collection note script. Targets a pool and calls the pool's `collect`
once per asset key. Storage is `6 + 4n` felts: pool id (2), serial (4), then one
asset key (4) per swept asset. The pool pays everything its vault holds above
the recorded reserves for each key to the registry's protocol account: the
protocol pot plus any credit outside the reserves for token0/token1, and the
whole balance for any other asset.

## Build

```bash
cargo miden build --release
```
