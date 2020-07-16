# rust-nomvec

vector implementation from [The Rustonomicon](https://doc.rust-lang.org/nomicon/) thats compatible with rust stable 1.45

## Testing

```sh
$ cargo test
```

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
