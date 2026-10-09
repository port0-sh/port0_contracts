// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

use miden::{Felt, component, component_storage};

#[component_storage]
struct PoolRegistryStorage;

#[component]
trait PoolRegistry {
    fn add(&self, a: Felt, b: Felt) -> Felt;
}

#[component]
impl PoolRegistry for PoolRegistryStorage {
    fn add(&self, a: Felt, b: Felt) -> Felt {
        a + b
    }
}
