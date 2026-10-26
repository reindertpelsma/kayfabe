# kayfabe user manual

Kayfabe shares an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it. It is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's unmodified driver on Windows or Linux; kayfabe reads what that driver asks for and has the real host GPU do the work, from an unprivileged host process. It is written in Rust.

There is no user install path yet; it is still being built. The security model is designed but not audited, and no public binary ships. Developers can build from source with `cargo build --workspace --release`; `cargo test` runs without a GPU.

This initial manual covers the existing Windows guest settings and FAQ. Source and current status: [repository README](https://github.com/reindertpelsma/kayfabe#readme).
