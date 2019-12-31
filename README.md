# rust-nomvec

vector implementation from [The Rustonomicon](https://doc.rust-lang.org/nomicon/) thats compatible with rust nightly 1.42

## Building / Testing

This repo uses a few `#![feature(_)]`s and those require you use the nightly rust channel.
You can enable this via `$ rustup override set nightly` in your project dir or via
[other precedence overrides](https://github.com/rust-lang/rustup#override-precedence).*

## Using

```rust
use nomvec::NomVec;

let mut cv: NomVec<i32> = NomVec::new();
cv.insert(0, 1);
cv.insert(0, 0);
cv.push(2);
for (i, x) in cv.iter().enumerate() {
    assert_eq!(i as i32, *x);
}
assert_eq!(cv.remove(0), 0);
assert_eq!(cv.len(), 2);
```
