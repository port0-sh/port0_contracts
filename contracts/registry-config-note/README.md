# registry_config_note

Registry config note script. Targets the registry and calls its
`set_register_fee`, changing the price of the register note. Storage is 7 felts:
registry id (2), the new register fee, and the register note's script root (4).

The registry accepts the note only when its sender equals the `admin` recorded at
creation, and only for the register note's own schedule entry.

## Build

```bash
cargo miden build --release
```
