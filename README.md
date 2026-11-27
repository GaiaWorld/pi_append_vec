# pi_append_vec

`AppendVec<T>` is a concurrent append-only vector. Writers serialize commits; readers access the published prefix without taking the commit lock. Appending does not move existing elements.

```toml
[dependencies]
pi_append_vec = "0.7"
```

```rust
use pi_append_vec::AppendVec;

let values = AppendVec::with_capacity(4);
let index = values.push("hello");
assert_eq!(values.get(index), Some(&"hello"));
assert_eq!(values.iter().copied().collect::<Vec<_>>(), ["hello"]);
```

`push` returns the index after publishing the value. Mutation through `get_mut` or `iter_mut`, compaction, and clearing require an exclusive borrow. `load` and `load_unchecked` require the caller to guarantee exclusive access to the target element.

Version 0.7.1 updates the underlying array and bucket crates without changing the public `AppendVec` API. See the [API documentation](https://docs.rs/pi_append_vec) for details.

Licensed under MIT or Apache-2.0.
