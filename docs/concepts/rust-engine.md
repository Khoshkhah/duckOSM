# The Rust engine

duckOSM is one Rust program, `duckosm`. The command line, the Python SDK, the Rust SDK and duckOSM Studio in the browser
all run the same Rust code, so they give the same rows and the same `edge_id`.

| You use | It runs |
|---|---|
| the command `duckosm` | the Rust program |
| the Python SDK (`import duckosm`) | the same program, through the module `duckosm._rust`. Python only calls it |
| the Rust SDK | the crates `duckosm-core`, `duckosm-build` and the others |
| [duckOSM Studio](https://khoshkhah.github.io/duckosm-studio/) | the same engine as WebAssembly, in the browser |

## Install

`pip install duckosm` installs the program and the Python SDK in one wheel, with nothing to compile. There are wheels for
Linux (x86_64, aarch64), macOS (x86_64, arm64) and Windows (x64). On another system, `pip` builds it from the source, and
that needs a [Rust toolchain](https://rustup.rs). [Install](../install.md).

## Speed

| Area | The network step |
|---|---|
| Estonia (three modes) | 26 s, 1.4 GB of memory |

The speed of each output: [Compare](../compare/index.md).

The engine reference and modules: [Rust SDK](../reference/rust.md).
