# mint_note

Add-liquidity note script. Targets a pool, carries token0 and token1, and calls
the pool's `mint`. Storage is 11 felts: pool id, recipient, serial, tag,
deadline, `min_lp_out`.

## Build

```bash
cargo miden build --release
```
