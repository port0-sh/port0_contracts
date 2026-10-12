# burn_note

Remove-liquidity note script. Targets a pool, carries the pool's LP token, and
calls the pool's `burn`. Storage is 12 felts: pool id, recipient, serial, tag,
deadline, `min_amount0`, `min_amount1`.

## Build

```bash
cargo miden build --release
```
