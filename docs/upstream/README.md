# Upstream reference docs

These come from [fee[dB]ack](https://github.com/got-feedback/feedBack), which RiffBoard is forked from. They still describe the frontend plugin contracts, highway renderers, capabilities and the feedpak format accurately.

Where they mention the Python server (`server.py`, `lib/`, plugin `routes.py`, settings writes, uploads and so on), that no longer applies. RiffBoard's server is the Rust binary in `src/`, and it only serves what [../../CLAUDE.md](../../CLAUDE.md) describes.
