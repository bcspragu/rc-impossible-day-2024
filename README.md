# RC Impossible Day 2024

Running the zulip bot

```bash
cd zulipbot && cargo run
```

To build for production: `cargo build --release --target=x86_64-unknown-linux-musl`

## Directories

* `zulipbot` contains the Rust Zulip bot, which auto-generates [Zola](https://www.getzola.org/) blogs
* `homepage` contains a simple HTML home page, live at `hypertxt.io`

## TODO

- [ ] Figure out why posts are being received
  - Probably related to recent mucking around with heartbeats and stuff
  - Generally make the logic match the Python SDK
- [ ] Update the `homepage` with new instructions about metadata tags
- [ ] Don't allow new users to take over existing domains :grimacing:
- [ ] Allow users to overwrite blog config stuff (`config.toml` mostly, but other directory creation and whatnot shouldn't fail)
- [ ] Test editing messages
