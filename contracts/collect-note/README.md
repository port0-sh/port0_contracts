# collect_note

Pool protocol-fee collection note script. Targets a pool and calls the pool's
`collect`, which moves the protocol pot to the registry's protocol account.
Storage is 4 felts: serial.

## Build

```bash
cargo miden build --release
```
