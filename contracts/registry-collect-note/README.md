# registry_collect_note

Registry fee collection note script. Targets the registry and calls its
`collect` once per asset key, moving the registry's whole balance of each named
asset to the protocol account. Storage is `6 + 4n` felts: registry id (2),
serial (4), then one asset key (4) per swept asset.

## Build

```bash
cargo miden build --release
```
