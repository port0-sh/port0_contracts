# registry_collect_note

Registry fee collection note script. Targets the registry and calls its
`collect`, which moves the registry's whole balance of one asset to the
protocol account. Storage is 10 felts: registry id, asset key (4), serial (4).

## Build

```bash
cargo miden build --release
```
