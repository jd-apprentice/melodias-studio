![melodias-studio](docs/assets/banner.png)

# melodias-studio

A Rust TUI that generates 8-bit thumbnails and original three to four minute instrumental chiptunes for a YouTube channel, using local models.

A reference track can guide the timbre. The thumbnail and the music are generated from scratch, not converted from the source. Each run produces three thumbnail candidates and three audio drafts, then pauses for you to pick the pair.

## Quick Start

Requires Rust 1.85+, `yt-dlp`, `ffmpeg`, `ffprobe` and three local HTTP services. See [setup](docs/setup.md) for the service contract and an example deployment.

```bash
git clone https://github.com/jd-apprentice/melodias-studio.git
cd melodias-studio
make install
melodias
```

## Docs

| Topic | Where |
| --- | --- |
| Requirements, install and server scripts | [docs/setup.md](docs/setup.md) |
| TUI, headless run, output and screenshots | [docs/usage.md](docs/usage.md) |
| Environment, `config.toml` and lifecycle commands | [docs/configuration.md](docs/configuration.md) |
| Pipeline, diagram and design notes | [docs/architecture/README.md](docs/architecture/README.md) |
| Adversary review and video assembly (design) | [docs/adversary-review.md](docs/adversary-review.md) |
| Roadmap | [docs/roadmap.md](docs/roadmap.md) |
| Project rules for agents | [AGENTS.md](AGENTS.md) |
| Commands | [Makefile](Makefile) |

## Contributing

PRs welcome. Run `make check` before opening one.

## License

MIT. The license field lives in [`Cargo.toml`](Cargo.toml).
