# async-wintp - An async runtime that uses Windows threadpool.

```rust
fn main() {
    async_wintp::runtime::Builder::new()
        .build()
        .expect("failed to build runtime")
        .block_on(async_main())
}

async fn async_main() {
    println!("Hello, world!");
}
```
