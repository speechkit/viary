// Voice engine (Engine artboard): local sherpa-onnx models from folders the
// user picks, OpenAI and DashScope with keys in the Keychain, and input.

import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { btn, btnPrimary, ErrorBanner, Field, H2, input, Seg, Select, Tag, useDraft } from "../components/Controls";
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
import { isMac, KEY_STORE, THIS_COMPUTER } from "../lib/platform";

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
  if (family && !family.nativePunctuation) tags.push(s.settings.punctModel ? "+ Paragraph punctuation" : "No punctuation");
  if (family?.nativePunctuation && s.punctLayout?.startsWith("CT-Transformer")) tags.push("+ Paragraph punctuation");
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
        <span className="flex min-w-0 flex-col">
          <span className="truncate text-[15px] font-semibold">{family?.label ?? model.family}</span>
          <span className="truncate font-mono text-[11px] text-faint" title={model.path}>
            {model.name}
          </span>
        </span>
        <span
          className="size-[18px] shrink-0 rounded-full"
          style={{ border: checked ? "5px solid #1C1B18" : "1.5px solid #CFC8B8" }}
        />
      </div>
      <span className="pointer-events-none text-[13px] leading-[1.45] text-muted">
        {family?.description}
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
export function AddModel({ onError }: { onError: (e: string) => void }) {
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
          Stored in {KEY_STORE}
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
        {isMac ? "Save to Keychain" : "Save key"}
      </button>
      {replacing && (
        <button type="button" className={btn} onClick={() => setReplacing(false)}>
          Cancel
        </button>
      )}
    </form>
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

      <ErrorBanner error={error} onDismiss={() => setError("")} />

      <H2>On {THIS_COMPUTER}</H2>
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
          caption="Adds punctuation to the whole dictation after you release the key. Choose CT-Transformer (Chinese and English) to replace pause-based sentence breaks from local engines such as SenseVoice and FunASR-Nano."
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
