# RC Impossible Day 2024

This is our (Brandon + Russell) RC Impossible Day 2024 project - a tool that turns Zulip messages directly into blog posts!

See `homepage/index.html` for usage details.

## Technical Details

* `zulipbot/` contains the Rust Zulip bot, which auto-generates [Zola](https://www.getzola.org/) blogs
* `homepage/` contains a simple HTML home page, live at `hypertxt.io`

## Running

Running the zulip bot

```bash
cd zulipbot && cargo run
```

To build for production: `cargo build --release --target=x86_64-unknown-linux-musl`

## TODO

- [ ] Allow users to overwrite blog config stuff (`config.toml` mostly, but other directory creation and whatnot shouldn't fail)
- [ ] Test editing messages
- [ ] Add image support
- [ ] Figure out if/how to backfill things
  - For when we add new features and want to fix old posts
- [x] Figure out why posts aren't being received
  - Probably related to recent mucking around with heartbeats and stuff
  - Generally make the logic match the Python SDK
- [x] Update the `homepage` with new instructions about metadata tags
- [x] Don't allow new users to take over existing domains :grimacing:
