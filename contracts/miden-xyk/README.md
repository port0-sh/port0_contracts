# miden-xyk

Pool account component for the MidenXYK constant-product AMM. Holds the token
asset keys, the LP supply, the two fee rates, the P2ID root, the registry
pointer, the protocol pot, and the price oracle. The LP token key is derived
from the pool's account id.

## Build

```bash
cargo miden build --release
```
