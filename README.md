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

Recognition results stay as `Transcript` through dictation, Retry, and History. Once recording ends, punctuation runs once over the full dictation, then dictionary replacements apply to the joined text, so they can span recognition segments. The raw text is saved before these edits.

Punctuation: an optional sherpa-onnx punctuation model runs once after the session, so "Use raw" can put back the recognizer's own text. Engines without punctuation (Streaming Zipformer) get the model's output for the whole text. Local engines that punctuate natively (SenseVoice, FunASR-Nano) end every recognition segment as a sentence, so a pause mid-thought becomes `。`. With a CT-Transformer (Chinese and English) model, the model reads the whole dictation but decides only what goes at each boundary between segments: nothing, a comma, or a sentence end, which keeps the recognizer's own mark (`？`, `！`). Text inside a segment, including its punctuation, numbers, and addresses, stays as recognized, and English boundaries get ASCII marks. Where the model's output does not match the text (it sometimes drops words), that boundary keeps its native mark. Cloud engines keep their native punctuation. The English-only CNN-BiLSTM model does not replace native punctuation.

SenseVoice waits for 1 second of silence before ending an utterance, keeping brief thinking pauses together to reduce unwanted sentence breaks. Releasing the hold-to-talk key flushes the remaining speech immediately.

For local Chinese paragraph punctuation, download and unpack the [official CT-Transformer int8 model](https://k2-fsa.github.io/sherpa/onnx/punctuation/pretrained_models.html#sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12-int8), then select its folder under Voice engine → Punctuation. The model runs locally, without sending transcripts to a service. One hold-and-release is one completed dictation; silence within it does not trigger final punctuation.

Dictionary priority: Strong words are prioritized when fitting model prompts. speechkit 0.5 accepts transducer hotwords as plain phrases, so each phrase uses the backend's default boost.

Engines such as SenseVoice do not support recognition hotwords. With these engines, set **Write as** to the correct spelling and **When I say** to the incorrect spelling from History (for example, `通义千问` and `通易千问`). A word without an incorrect spelling only restores capitalization; it cannot correct a mishearing.

For an independent polish service, choose **Custom** under **Polish & tone** to connect to an OpenAI-compatible chat API. Enter its API base URL and chat model ID, and an API key if required. **Test connection** uses the current form without saving it; **Save** stores the settings and puts the key in a separate macOS Keychain entry. Leave the key field blank to keep a stored key, or use **Remove key** for an unauthenticated server. OpenAI and DashScope presets continue to reuse Voice engine credentials. Only transcripts are sent for polishing; the destination is shown in the model card.

## Windows and Linux

The parts that talk to the system live in `src-tauri/src/platform/`, one module per system with the same items; everything else is shared.

- **Windows** (in progress): hold Right Alt or Ctrl + Win, watched by a low-level keyboard hook that only listens. Text goes in with Ctrl+V through `SendInput`, with the clipboard saved and restored and kept out of clipboard history. Apps running as administrator do not accept Viary's keys, so their text stays on the clipboard. API keys are in Windows Credential Manager. Left-click the tray icon for the flyout, right-click for the menu.
- **Linux** (in progress, GNOME): the talk shortcut is Ctrl+Alt+Space, held to talk. On Wayland, Viary's GNOME Shell extension (`gnome-extension/`, installed from the setup window; GNOME starts it at the next login) grabs the shortcut, types the text, names the focused app, and draws the pill; Viary needs it there. On X11, Viary grabs the shortcut itself and types with XTest. API keys are in the GNOME keyring.

CI builds and tests on macOS, Windows, and Ubuntu 26.04 (`.github/workflows/ci.yml`). On Ubuntu it also starts a headless GNOME Shell on Wayland with Viary's extension and checks the extension (`ci/gnome-wayland.sh`). `/preview.html?os=windows` renders the windows as they look on Windows.

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
