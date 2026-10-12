# pool_registry

Registry account component for the port0.sh constant-product AMM. Holds the
accepted pool code commitments, the registered pool set, the ordered asset pair
index, the protocol fee destination and rate, the allowed LP fee tiers, and the
P2ID root of the register-fee payout note. `register` verifies a pool's code
commitment and identity by foreign procedure invocation before listing it.

## Build

```bash
cargo miden build --release
```
