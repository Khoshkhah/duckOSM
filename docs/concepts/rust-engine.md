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
| Estonia | 19 s, 1.5 GB of memory |

The speed of each output: [Compare](../compare/index.md).

## Not ported to Rust yet

These commands still run code that is not ported to Rust yet. They give the same tables, and each port keeps them equal.

| Command | Not ported yet |
|---|---|
| `duckosm build` with a config file | the validation, the report and the multimodal graph. Its network steps run in Rust. `duckosm FILE` is the whole build in Rust |
| `duckosm levels` | the whole command |

The engine reference and modules: [Rust SDK](../reference/rust.md).
