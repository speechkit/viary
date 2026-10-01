# Viary

A macOS menu bar dictation app: hold a key, speak, release, and the text is pasted where your cursor is. Built with Tauri 2, React, and Tailwind on [speechkit](https://crates.io/crates/speechkit) 0.5.0.

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
| Microphone: `Microphone::capture` and `Capture::listen` retain audio for Retry and buffer it while a cloud session connects. `Listening::level` and `LiveTranscript` supply the waveform and live text through one observer; key-up calls `stop` immediately, then `finish` waits on a worker | `src-tauri/src/recording.rs` |
| The dictation state machine: listen, transcribe, paste, Undo / Use raw, and Retry / switch engine with the kept audio | `src-tauri/src/dictation.rs` |
| Engines: `sherpa::inspect` detects picked folders without native code; streaming vs. offline (behind silero VAD) load with `AsrConfig::load`, plus OpenAI (file or Realtime) and DashScope | `src-tauri/src/engines.rs` |
| Switching engines between dictations, debounced, keeping the old one until the new one loads | `src-tauri/src/reload.rs` |
| API keys in the Keychain (`app.viary.api-key`); never sent to the web view | `src-tauri/src/keychain.rs` |
| Pasting: the pasteboard is saved, ⌘V posted, then restored; without a focused text field the text stays on the clipboard | `src-tauri/src/macos/` |
| History and recordings, kept for the chosen number of days | `src-tauri/src/history.rs` |
| Dictionary: hotwords for transducers (with `bpe.vocab`, or Chinese-character models), a prompt for Qwen3-ASR, FunASR-Nano and OpenAI file mode, and "When I say" replacements after recognition for every engine | `src-tauri/src/dictionary.rs` |
| Polish & tone: an OpenAI-compatible chat model (a local server such as Ollama, a custom endpoint with its own optional Keychain API key, OpenAI, or DashScope) rewrites the text before it is pasted, with a tone per app; if it fails, the text goes in as recognized | `src-tauri/src/polish.rs` |

Recognition results stay as `Transcript` through dictation, Retry, and History; dictionary and punctuation processing update its segment text before `.text()` joins it. The raw text is saved before these edits.

Punctuation: engines that punctuate natively (SenseVoice, cloud) are used as-is. For the others, an optional sherpa-onnx punctuation model runs after the session, so "Use raw" can put back the recognizer's own text.

SenseVoice waits for 1 second of silence before ending an utterance, keeping brief thinking pauses together to reduce unwanted sentence breaks. Releasing the hold-to-talk key flushes the remaining speech immediately.

Dictionary priority: Strong words are prioritized when fitting model prompts. speechkit 0.5 accepts transducer hotwords as plain phrases, so each phrase uses the backend's default boost.

Engines such as SenseVoice do not support recognition hotwords. With these engines, set **Write as** to the correct spelling and **When I say** to the incorrect spelling from History (for example, `通义千问` and `通易千问`). A word without an incorrect spelling only restores capitalization; it cannot correct a mishearing.

For an independent polish service, choose **Custom** under **Polish & tone** to connect to an OpenAI-compatible chat API. Enter its API base URL and chat model ID, and an API key if required. **Test connection** uses the current form without saving it; **Save** stores the settings and puts the key in a separate macOS Keychain entry. Leave the key field blank to keep a stored key, or use **Remove key** for an unauthenticated server. OpenAI and DashScope presets continue to reuse Voice engine credentials. Only transcripts are sent for polishing; the destination is shown in the model card.

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
