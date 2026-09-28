# Viary

A macOS menu bar dictation app: hold a key, speak, release, and the text is pasted where your cursor is. Built with Tauri 2, React, and Tailwind on [speechkit](https://crates.io/crates/speechkit) 0.3.

## Run

```sh
npm install
npm run tauri dev
```

The first run opens the main window. Then:

1. **Voice engine**: add an unpacked sherpa-onnx model folder (Viary detects its layout), or add an OpenAI or DashScope key and model name. Offline models need `silero_vad.onnx`; Viary picks it up when it sits next to the model folder.
2. **Input Monitoring**, so Viary can see the hold-to-talk key, and **Accessibility**, so it can paste. Under `tauri dev`, macOS attributes these to the app that launched the binary (your terminal).
3. For the fn key, set System Settings › Keyboard › "Press 🌐 key to" to "Do Nothing", or choose right ⌥ or right ⌘ under Settings.

## How it works

| Piece | Where |
|---|---|
| Hold-to-talk key: a listen-only CGEventTap on modifier flags | `src-tauri/src/macos/hotkey.rs` |
| Microphone: `speechkit::io::Microphone::listen` feeds a small tap backend, so each chunk is recorded (for Retry), metered (for the waveform), and pushed into the engine session. Audio waits in a backlog while a cloud session connects | `src-tauri/src/tap.rs` |
| The dictation state machine: listen, transcribe, paste, Undo / Use raw, and Retry / switch engine with the kept audio | `src-tauri/src/dictation.rs` |
| Engines: `SherpaAsrConfig::validate` detects picked folders; streaming vs. offline (behind silero VAD) load with `SherpaAsr`, plus OpenAI (file or Realtime) and DashScope | `src-tauri/src/engines.rs` |
| Switching engines between dictations, debounced, keeping the old one until the new one loads | `src-tauri/src/reload.rs` |
| API keys in the Keychain (`app.viary.api-key`); never sent to the web view | `src-tauri/src/keychain.rs` |
| Pasting: the pasteboard is saved, ⌘V posted, then restored; without a focused text field the text stays on the clipboard | `src-tauri/src/macos/` |
| History and recordings, kept for the chosen number of days | `src-tauri/src/history.rs` |
| Dictionary: hotwords for transducers (with `bpe.vocab`, or Chinese-character models), a prompt for Qwen3-ASR, FunASR-Nano and OpenAI file mode, and "When I say" replacements after recognition for every engine | `src-tauri/src/dictionary.rs` |
| Polish & tone: an OpenAI-compatible chat model (a local server such as Ollama, OpenAI, or DashScope) rewrites the text before it is pasted, with a tone per app; if it fails, the text goes in as recognized | `src-tauri/src/polish.rs` |

Punctuation: engines that punctuate natively (SenseVoice, cloud) are used as-is. For the others, an optional sherpa-onnx punctuation model runs after the session, so "Use raw" can put back the recognizer's own text.

## Tests

```sh
cd src-tauri
cargo test                 # no models, microphone, or network needed
cargo test -- --ignored    # real models from ~/.cache/speechkit/models (or $SPEECHKIT_MODELS)
```

The ignored tests need `sherpa-onnx-streaming-zipformer-en-2023-06-26`, `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17`, and `silero_vad.onnx`.

`/preview.html?w=main|popover|pill` on the Vite dev server renders the windows against a mocked backend, for comparing them with the design.

## Not yet

Hands-free (double-tap), speak-to-edit, learning dictionary words from your corrections, and crash isolation through `speechkit-worker`.
