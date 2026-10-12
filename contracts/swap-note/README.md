# swap_note

Swap note script. Targets a pool, carries token0 or token1, and calls the
pool's `swap`. Storage is 11 felts: pool id, recipient, serial, tag, deadline,
`min_amount_out`.

## Build

```bash
cargo miden build --release
```
