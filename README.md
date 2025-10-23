# OpenVST3 Complete (v1.0.0)

This workspace provides a **functional** VST3 host stack in Rust by compiling a small **C++ shim**
against the official VST3 SDK at build time. **You must** set `VST3_SDK_DIR` to your local clone.

> We do not distribute Steinberg headers or code. You accept their license when using the SDK.

## Build prerequisites
- Rust 1.75+
- `libclang` (common on most distros) and a C++17 compiler
- Steinberg VST3 SDK cloned locally:
  ```bash
  git clone https://github.com/steinbergmedia/vst3sdk ~/dev/vst3sdk
  export VST3_SDK_DIR=~/dev/vst3sdk
  ```

## Build and run
```bash
cargo build --workspace

# Run the example host; plugin path is the .so inside a .vst3 bundle:
cargo run -p host-cli --   --plugin /path/to/MyPlug.vst3/Contents/x86_64-linux/MyPlug.so   --blocks 64 --block-size 256 --sr 48000 --in 2 --out 2
```
