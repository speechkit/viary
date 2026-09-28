// Voice engine (Engine artboard): local sherpa-onnx models from folders the
// user picks, OpenAI and DashScope with keys in the Keychain, and input.

import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icons";
import {
  api,
  errorText,
  formatBytes,
  type DashScopeSettings,
  type FamilyInfo,
  type LocalModel,
  type ModelInspection,
  type OpenAiSettings,
  type Provider,
  type Snapshot,
} from "../lib/ipc";

const btn =
  "inline-flex h-8 shrink-0 items-center whitespace-nowrap gap-2 rounded-lg border border-edge bg-white px-3 text-[13px] font-medium text-ink disabled:opacity-50";
const btnPrimary =
  "inline-flex h-8 shrink-0 items-center whitespace-nowrap gap-2 rounded-lg border border-ink bg-ink px-3 text-[13px] font-medium text-white disabled:opacity-40";
const input =
  "h-[34px] rounded-lg border border-edge bg-white px-2.5 text-[13px] text-ink outline-none placeholder:text-faint focus:border-ink";

/**
 * The design's dropdown: a white 34 px button, and a menu card drawn in the
 * page (the native select opens the system menu, which ignores the design).
 */
function Select<T extends string | number>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  const [open, setOpen] = useState(false);
  const [up, setUp] = useState(false);
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const current = options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  const show = () => {
    const box = root.current?.getBoundingClientRect();
    // Open upward when the menu would run off the bottom of the window.
    setUp(!!box && window.innerHeight - box.bottom < options.length * 34 + 24);
    setActive(Math.max(0, options.findIndex((o) => o.value === value)));
    setOpen(true);
  };
  const pick = (option: { value: T }) => {
    setOpen(false);
    if (option.value !== value) onChange(option.value);
  };
  const onKey = (e: React.KeyboardEvent) => {
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        show();
      }
      return;
    }
    if (e.key === "Escape") setOpen(false);
    else if (e.key === "ArrowDown") setActive((i) => Math.min(options.length - 1, i + 1));
    else if (e.key === "ArrowUp") setActive((i) => Math.max(0, i - 1));
    else if (e.key === "Enter" || e.key === " ") pick(options[active]);
    else return;
    e.preventDefault();
  };

  return (
    <div ref={root} className="relative shrink-0" onKeyDown={onKey}>
      <button
        type="button"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => (open ? setOpen(false) : show())}
        className={
          "flex h-[34px] max-w-[300px] items-center gap-2 rounded-lg border bg-white pr-2.5 pl-3 text-[13px] font-medium text-ink " +
          (open ? "border-ink" : "border-edge hover:border-stone")
        }
      >
        <span className="truncate">{current?.label}</span>
        <svg viewBox="0 0 24 24" width={14} height={14} aria-hidden="true" className="shrink-0 text-muted" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      {open && (
        <ul
          role="listbox"
          aria-label={label}
          className={
            "absolute right-0 z-20 m-0 flex max-h-[260px] min-w-full list-none flex-col overflow-y-auto rounded-[10px] bg-white p-1 shadow-[0_12px_32px_rgba(28,27,24,0.18),0_0_0_1px_rgba(28,27,24,0.08)] " +
            (up ? "bottom-[calc(100%+6px)]" : "top-[calc(100%+6px)]")
          }
        >
          {options.map((option, i) => {
            const selected = option.value === value;
            return (
              <li
                key={String(option.value)}
                role="option"
                aria-selected={selected}
                onMouseEnter={() => setActive(i)}
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(option);
                }}
                className={
                  "flex h-8 cursor-pointer items-center gap-2 rounded-md pr-3 pl-2 text-[13px] whitespace-nowrap " +
                  (i === active ? "bg-sand" : "")
                }
              >
                <span className="w-4 shrink-0 text-ink">{selected && <Icon name="check" size={14} />}</span>
                <span className={selected ? "font-medium" : ""}>{option.label}</span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function H2({ children }: { children: React.ReactNode }) {
  return <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">{children}</h2>;
}

function Tag({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-[22px] items-center rounded-full bg-paper px-2 text-[11px] font-medium text-muted">
      {children}
    </span>
  );
}

function Seg<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: [T, string][];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex gap-0.5 rounded-[9px] bg-sand p-[3px]">
      {options.map(([v, text]) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={v === value}
          onClick={() => onChange(v)}
          className={
            "h-7 rounded-[7px] px-3 text-[13px] font-medium whitespace-nowrap " +
            (v === value ? "bg-white text-ink shadow-[0_1px_2px_rgba(0,0,0,.12)]" : "text-muted")
          }
        >
          {text}
        </button>
      ))}
    </div>
  );
}

/** A local model's line under its card: installed, in use, loading, failed. */
function modelState(s: Snapshot, id: string, model: LocalModel, family?: FamilyInfo): { text: string; color: string } {
  const { engine } = s;
  if (engine.loading === id) return { text: "Loading…", color: "text-blue" };
  if (engine.failed === id && engine.error) return { text: engine.error, color: "text-rust" };
  const size = formatBytes(model.sizeBytes);
  if (engine.active?.id === id) return { text: `Installed · ${size} · in use`, color: "text-teal" };
  if (family?.needsVad && !s.settings.vadModel) return { text: `Needs silero_vad.onnx · ${size}`, color: "text-amber" };
  return { text: `Installed · ${size}`, color: "text-teal" };
}

function LocalCard({ s, model, onError }: { s: Snapshot; model: LocalModel; onError: (e: string) => void }) {
  const id = `local:${model.id}`;
  const family = s.families.find(([mid]) => mid === model.id)?.[1];
  const checked = s.engine.active?.id === id;
  const state = modelState(s, id, model, family);
  const tags = [...(family?.tags ?? [])];
  if (family && !family.nativePunctuation) tags.push(s.settings.punctModel ? "+ Punctuation model" : "No punctuation");
  return (
    <div
      className={
        "relative flex flex-col gap-2.5 rounded-[14px] bg-white px-[18px] py-4 " +
        (checked ? "border-2 border-ink" : "border border-line")
      }
    >
      <button
        type="button"
        role="radio"
        aria-checked={checked}
        onClick={() => {
          // Picking the engine in use again would reload the whole model.
          if (checked || s.engine.loading === id) return;
          api.setActiveEngine(id).catch((e) => onError(errorText(e)));
        }}
        className="absolute inset-0 rounded-[14px]"
        aria-label={`Use ${model.name}`}
      />
      <div className="pointer-events-none flex items-center justify-between gap-2">
        <span className="truncate text-[15px] font-semibold" title={model.path}>
          {model.name}
        </span>
        <span
          className="size-[18px] shrink-0 rounded-full"
          style={{ border: checked ? "5px solid #1C1B18" : "1.5px solid #CFC8B8" }}
        />
      </div>
      <span className="pointer-events-none text-[13px] leading-[1.45] text-muted">
        <b className="font-medium text-ink">{family?.label ?? model.family}.</b> {family?.description}
      </span>
      <div className="pointer-events-none flex flex-wrap gap-1.5">
        {tags.map((t) => (
          <Tag key={t}>{t}</Tag>
        ))}
      </div>
      <div className="flex items-end justify-between gap-2">
        <span className={`pointer-events-none line-clamp-3 text-xs font-medium ${state.color}`} title={state.text}>
          {state.text}
        </span>
        <button
          type="button"
          className="relative shrink-0 text-xs text-faint hover:text-rust"
          onClick={() => api.removeLocalModel(model.id)}
        >
          Remove
        </button>
      </div>
    </div>
  );
}

/** Adds a model folder: detect its layout, and ask for the family only when the files cannot tell. */
function AddModel({ onError }: { onError: (e: string) => void }) {
  const [found, setFound] = useState<ModelInspection | null>(null);
  const [family, setFamily] = useState<string>("");
  const [busy, setBusy] = useState(false);

  const add = async (inspection: ModelInspection, familyId: string) => {
    setBusy(true);
    try {
      const id = await api.addLocalModel(inspection.path, familyId);
      setFound(null);
      await api.setActiveEngine(id);
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const pick = async () => {
    onError("");
    const dir = await open({ directory: true, title: "Choose a sherpa-onnx model folder" });
    if (typeof dir !== "string") return;
    try {
      const inspection = await api.inspectModel(dir);
      if (inspection.families.length === 1) {
        await add(inspection, inspection.families[0].id);
      } else {
        setFamily(inspection.families[0].id);
        setFound(inspection);
      }
    } catch (e) {
      onError(errorText(e));
    }
  };

  if (found) {
    return (
      <div className="flex flex-col gap-3 rounded-[14px] border-2 border-blue bg-white px-[18px] py-4">
        <span className="truncate text-[15px] font-semibold" title={found.path}>
          {found.name}
        </span>
        <span className="text-[13px] leading-[1.45] text-muted">
          A {found.layout} model · {formatBytes(found.sizeBytes)}. The files do not say which kind; choose one.
        </span>
        <div role="radiogroup" aria-label="Model family" className="flex flex-col gap-1">
          {found.families.map((f) => (
            <label key={f.id} className="flex items-center gap-2 text-[13px]">
              <input type="radio" name="family" checked={family === f.id} onChange={() => setFamily(f.id)} />
              <span className="font-medium">{f.label}</span>
              {f.streaming && <Tag>Live preview</Tag>}
            </label>
          ))}
        </div>
        <div className="flex gap-2">
          <button type="button" className={btnPrimary} disabled={busy} onClick={() => add(found, family)}>
            Add and use
          </button>
          <button type="button" className={btn} onClick={() => setFound(null)}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  return (
    <button
      type="button"
      onClick={pick}
      className="flex min-h-[172px] flex-col items-center justify-center gap-2 rounded-[14px] border border-dashed border-stone bg-transparent px-[18px] py-4 text-muted hover:border-ink hover:text-ink"
    >
      <Icon name="folder" size={20} />
      <span className="text-sm font-medium">Add a model folder…</span>
      <span className="max-w-[220px] text-center text-xs leading-[1.45] text-faint">
        An unpacked sherpa-onnx recognition model. Viary detects its layout.
      </span>
    </button>
  );
}

function PathRow({
  title,
  caption,
  value,
  choose,
  clear,
}: {
  title: string;
  caption: string;
  value: string | null;
  choose: () => void;
  clear: () => void;
}) {
  return (
    <div className="flex items-center gap-3.5 border-b border-hair px-[18px] py-3 last:border-b-0">
      <div className="flex min-w-0 grow flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="truncate text-[13px] leading-[1.45] text-muted" title={value ?? undefined}>
          {value ?? caption}
        </span>
      </div>
      {value && (
        <button type="button" className={btn} onClick={clear}>
          Clear
        </button>
      )}
      <button type="button" className={btn} onClick={choose}>
        Choose…
      </button>
    </div>
  );
}

function KeyField({ s, provider, onError }: { s: Snapshot; provider: Provider; onError: (e: string) => void }) {
  const stored = provider === "openAi" ? s.keys.openAi : s.keys.dashScope;
  const [key, setKey] = useState("");
  const [replacing, setReplacing] = useState(false);
  if (stored && !replacing) {
    return (
      <div className="flex items-center gap-2">
        <span className="inline-flex items-center gap-1.5 text-[13px] whitespace-nowrap text-teal-ink">
          <Icon name="shield" size={14} />
          Stored in the Keychain
        </span>
        <div className="grow" />
        <button type="button" className={btn} onClick={() => setReplacing(true)}>
          Replace
        </button>
        <button type="button" className={btn} onClick={() => api.deleteApiKey(provider).catch((e) => onError(errorText(e)))}>
          Remove
        </button>
      </div>
    );
  }
  const save = () =>
    api
      .saveApiKey(provider, key)
      .then(() => {
        setKey("");
        setReplacing(false);
      })
      .catch((e) => onError(errorText(e)));
  return (
    <form
      className="flex gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <input
        type="password"
        autoComplete="off"
        spellCheck={false}
        aria-label="API key"
        placeholder="Paste your API key"
        className={`${input} grow`}
        value={key}
        onChange={(e) => setKey(e.target.value)}
      />
      <button type="submit" className={btnPrimary} disabled={!key.trim()}>
        Save to Keychain
      </button>
      {replacing && (
        <button type="button" className={btn} onClick={() => setReplacing(false)}>
          Cancel
        </button>
      )}
    </form>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="grid grid-cols-[96px_1fr] items-center gap-3 text-[13px]">
      <span className="text-muted">{label}</span>
      {children}
    </label>
  );
}

function CloudCard({
  s,
  id,
  name,
  caption,
  ready,
  children,
}: {
  s: Snapshot;
  id: "openai" | "dashscope";
  name: string;
  caption: string;
  ready: boolean;
  children: React.ReactNode;
}) {
  const active = s.engine.active?.id === id;
  const hasKey = id === "openai" ? s.keys.openAi : s.keys.dashScope;
  const [expanded, setExpanded] = useState(!hasKey ? false : !ready);
  let state: { text: string; color: string } | null = null;
  if (s.engine.loading === id) state = { text: "Connecting…", color: "text-blue" };
  else if (s.engine.failed === id && s.engine.error) state = { text: s.engine.error, color: "text-rust" };
  else if (active) state = { text: `In use · ${s.engine.active?.name}`, color: "text-teal" };
  return (
    <div className={"flex flex-col rounded-[14px] bg-white " + (active ? "border-2 border-ink" : "border border-line")}>
      <div className="flex items-center gap-3.5 px-[18px] py-3.5">
        <Icon name="cloud" size={20} />
        <div className="flex min-w-0 grow flex-col gap-0.5">
          <span className="text-sm font-semibold">{name}</span>
          <span className={`line-clamp-2 text-[13px] leading-[1.45] ${state?.color ?? "text-muted"}`} title={state?.text}>
            {state?.text ?? caption}
          </span>
        </div>
        {!hasKey ? (
          <button type="button" className={btn} onClick={() => setExpanded(true)}>
            Add API key
          </button>
        ) : (
          <>
            <button type="button" className={btn} onClick={() => setExpanded(!expanded)}>
              {expanded ? "Done" : "Configure"}
            </button>
            {!active && (
              <button type="button" className={btnPrimary} disabled={!ready} onClick={() => api.setActiveEngine(id)}>
                Use
              </button>
            )}
          </>
        )}
      </div>
      {expanded && <div className="flex flex-col gap-3 border-t border-hair px-[18px] py-3.5">{children}</div>}
    </div>
  );
}

/**
 * A form's unsaved copy of `saved`. It follows `saved` only when the stored
 * value really changes: every refresh brings a new object, and resetting
 * on identity would throw away what the user is typing.
 */
function useDraft<T>(saved: T): [T, (value: T) => void, boolean] {
  const key = JSON.stringify(saved);
  const [draft, setDraft] = useState<T>(saved);
  useEffect(() => setDraft(JSON.parse(key) as T), [key]);
  return [draft, setDraft, JSON.stringify(draft) !== key];
}

function OpenAiForm({ s, onError }: { s: Snapshot; onError: (e: string) => void }) {
  const [draft, setDraft, dirty] = useDraft<OpenAiSettings>(s.settings.openai);
  return (
    <>
      <KeyField s={s} provider="openAi" onError={onError} />
      <Field label="Model">
        <input
          className={input}
          spellCheck={false}
          placeholder="The transcription model to use"
          value={draft.model}
          onChange={(e) => setDraft({ ...draft, model: e.target.value })}
        />
      </Field>
      <Field label="Mode">
        <div>
          <Seg
            label="OpenAI mode"
            value={draft.mode}
            options={[
              ["file", "On release"],
              ["realtime", "Realtime"],
            ]}
            onChange={(mode) => setDraft({ ...draft, mode })}
          />
        </div>
      </Field>
      {draft.mode === "file" && (
        <Field label="Base URL">
          <input
            className={input}
            spellCheck={false}
            value={draft.baseUrl}
            onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
          />
        </Field>
      )}
      <div className="flex justify-end">
        <button type="button" className={btnPrimary} disabled={!dirty} onClick={() => api.setOpenAi(draft).catch((e) => onError(errorText(e)))}>
          Save
        </button>
      </div>
    </>
  );
}

function DashScopeForm({ s, onError }: { s: Snapshot; onError: (e: string) => void }) {
  const [draft, setDraft, dirty] = useDraft<DashScopeSettings>(s.settings.dashscope);
  return (
    <>
      <KeyField s={s} provider="dashScope" onError={onError} />
      <Field label="Model">
        <input
          className={input}
          spellCheck={false}
          placeholder="A real-time recognition model"
          value={draft.model}
          onChange={(e) => setDraft({ ...draft, model: e.target.value })}
        />
      </Field>
      <Field label="Region">
        <div>
          <Seg
            label="DashScope region"
            value={draft.region}
            options={[
              ["china", "Mainland China"],
              ["international", "International"],
            ]}
            onChange={(region) => setDraft({ ...draft, region })}
          />
        </div>
      </Field>
      <div className="flex justify-end">
        <button
          type="button"
          className={btnPrimary}
          disabled={!dirty}
          onClick={() => api.setDashScope(draft).catch((e) => onError(errorText(e)))}
        >
          Save
        </button>
      </div>
    </>
  );
}

export function EnginePage({ s }: { s: Snapshot }) {
  const [error, setError] = useState("");
  const [mics, setMics] = useState<{ name: string; isDefault: boolean }[]>([]);
  useEffect(() => {
    api.microphones().then(setMics, (e) => setError(errorText(e)));
  }, []);
  const { settings } = s;
  const defaultMic = mics.find((m) => m.isDefault);

  const chooseVad = async () => {
    const file = await open({ title: "Choose silero_vad.onnx", filters: [{ name: "ONNX model", extensions: ["onnx"] }] });
    if (typeof file === "string") api.setVadModel(file).catch((e) => setError(errorText(e)));
  };
  const choosePunct = async () => {
    const dir = await open({ directory: true, title: "Choose a sherpa-onnx punctuation model folder" });
    if (typeof dir === "string") api.setPunctModel(dir).catch((e) => setError(errorText(e)));
  };

  return (
    <main className="flex max-w-[1100px] flex-col gap-[18px] px-12 pt-9 pb-12">
      <div className="flex flex-col gap-1.5">
        <h1 className="m-0 font-serif text-[40px] font-normal">Voice engine</h1>
        <span className="text-[13px] leading-[1.45] text-muted">
          Choose where recognition runs. You can switch any time; the next dictation uses the new engine.
        </span>
      </div>

      {error && (
        <div role="alert" className="flex items-start gap-2 rounded-[10px] bg-rust-wash px-3 py-2.5 text-[13px] text-rust">
          <Icon name="warning" />
          <span className="grow">{error}</span>
          <button type="button" aria-label="Dismiss" onClick={() => setError("")}>
            <Icon name="close" size={14} />
          </button>
        </div>
      )}

      <H2>On this Mac</H2>
      <div role="radiogroup" aria-label="Recognition engine" className="grid grid-cols-3 gap-3.5">
        {settings.localModels.map((m) => (
          <LocalCard key={m.id} s={s} model={m} onError={setError} />
        ))}
        <AddModel onError={setError} />
      </div>
      <div className="overflow-hidden rounded-[14px] border border-line bg-white">
        <PathRow
          title="Voice activity detection"
          caption="silero_vad.onnx. Models that transcribe phrase by phrase need it to find pauses."
          value={settings.vadModel}
          choose={chooseVad}
          clear={() => api.setVadModel(null)}
        />
        <PathRow
          title="Punctuation"
          caption="A sherpa-onnx punctuation model, for engines that write none, such as Streaming Zipformer."
          value={settings.punctModel && `${settings.punctModel}${s.punctLayout ? ` · ${s.punctLayout}` : ""}`}
          choose={choosePunct}
          clear={() => api.setPunctModel(null)}
        />
        <div className="flex items-center gap-3.5 px-[18px] py-3">
          <div className="flex min-w-0 grow flex-col gap-0.5">
            <span className="text-sm font-medium">Run models on</span>
            <span className="text-[13px] leading-[1.45] text-muted">CoreML may use the GPU or the Neural Engine.</span>
          </div>
          <Seg
            label="Execution provider"
            value={settings.provider}
            options={[
              ["cpu", "CPU"],
              ["coreml", "CoreML"],
            ]}
            onChange={(provider) => api.setPreferences({ provider }).catch((e) => setError(errorText(e)))}
          />
        </div>
      </div>

      <H2>Cloud</H2>
      <div className="grid grid-cols-1 items-start gap-3.5 min-[1180px]:grid-cols-2">
        <CloudCard
          s={s}
          id="openai"
          name="OpenAI"
          caption="Realtime streaming or file transcription"
          ready={s.keys.openAi && !!settings.openai.model.trim()}
        >
          <OpenAiForm s={s} onError={setError} />
        </CloudCard>
        <CloudCard
          s={s}
          id="dashscope"
          name="DashScope"
          caption="Paraformer realtime, strong for Chinese"
          ready={s.keys.dashScope && !!settings.dashscope.model.trim()}
        >
          <DashScopeForm s={s} onError={setError} />
        </CloudCard>
      </div>

      <H2>Input &amp; reliability</H2>
      <div className="rounded-[14px] border border-line bg-white">
        <div className="flex items-center gap-3.5 border-b border-hair px-[18px] py-3">
          <span className="min-w-0 grow text-sm font-medium">Microphone</span>
          <Select
            label="Microphone"
            value={settings.microphone ?? ""}
            options={[
              { value: "", label: defaultMic ? `${defaultMic.name} (default)` : "System default" },
              ...mics.filter((m) => !m.isDefault).map((m) => ({ value: m.name, label: m.name })),
            ]}
            onChange={(name) => api.setPreferences({ microphone: name || null })}
          />
        </div>
        <div className="flex items-center gap-3.5 px-[18px] py-3">
          <div className="flex min-w-0 grow flex-col gap-0.5">
            <span className="text-sm font-medium">Keep recordings</span>
            <span className="text-[13px] leading-[1.45] text-muted">Needed for replay and re-transcribing from History.</span>
          </div>
          <Select
            label="Keep recordings"
            value={settings.keepRecordingsDays}
            options={[
              { value: 0, label: "Never" },
              { value: 7, label: "7 days" },
              { value: 30, label: "30 days" },
            ]}
            onChange={(days) => api.setPreferences({ keepRecordingsDays: days })}
          />
        </div>
      </div>
    </main>
  );
}
