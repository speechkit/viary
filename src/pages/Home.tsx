// Home: how to dictate, what is left to set up, and recent dictations.

import { Icon, Kbd } from "../components/Icons";
import { api, useHistory, type Snapshot } from "../lib/ipc";

type Go = (page: "home" | "history" | "engine" | "settings") => void;

function Step({ done, title, text, action }: { done: boolean; title: string; text: string; action?: React.ReactNode }) {
  return (
    <div className="flex items-center gap-4 border-b border-hair px-[22px] py-3.5 last:border-b-0">
      <span
        className={
          "inline-flex size-6 shrink-0 items-center justify-center rounded-full " +
          (done ? "bg-teal text-white" : "border-[1.5px] border-stone")
        }
      >
        {done && <Icon name="check" size={14} />}
      </span>
      <div className="flex min-w-0 grow flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-[13px] text-muted">{text}</span>
      </div>
      {!done && action}
    </div>
  );
}

const btn = "inline-flex h-9 shrink-0 items-center whitespace-nowrap gap-2 rounded-[9px] border border-edge bg-white px-3.5 text-sm font-medium text-ink";

export function HomePage({ s, go }: { s: Snapshot; go: Go }) {
  const [history] = useHistory();
  const items = history ?? [];
  const weekAgo = Date.now() - 7 * 86_400_000;
  const week = items.filter((h) => h.createdAt >= weekAgo && h.text && h.status !== "undone");
  const words = week.reduce((sum, h) => sum + h.words, 0);
  const seconds = week.reduce((sum, h) => sum + h.durationMs, 0) / 1000;
  const wpm = seconds > 5 ? Math.round(words / (seconds / 60)) : null;
  const setupDone = !!s.engine.active && s.permissions.inputMonitoring && s.permissions.accessibility;
  const today = new Date().toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });

  return (
    <main className="flex flex-col gap-6 px-12 pt-11 pb-10">
      <div className="flex items-end justify-between">
        <div className="flex flex-col gap-1.5">
          <span className="text-[13px] text-muted">{today}</span>
          <h1 className="m-0 font-serif text-[44px] font-normal">Talk, don't type.</h1>
        </div>
      </div>

      <div className="flex items-center gap-6 rounded-[14px] border border-line bg-white px-6 py-5">
        <Kbd large>{s.hotkeyName}</Kbd>
        <div className="flex flex-col gap-1">
          <span className="text-[15px] font-semibold">Hold to dictate</span>
          <span className="text-[13px] text-muted">
            Speak while holding {s.hotkeyName}; release to insert where your cursor is. Undo from the pill.
          </span>
        </div>
      </div>

      {!setupDone && (
        <section className="overflow-hidden rounded-[14px] border border-line bg-white" aria-label="Setup">
          <h2 className="m-0 border-b border-hair px-[22px] py-4 text-[15px] font-semibold">Get set up</h2>
          <Step
            done={!!s.engine.active}
            title="Choose a voice engine"
            text="A sherpa-onnx model folder on this Mac, or OpenAI or DashScope with your key."
            action={
              <button type="button" className={btn} onClick={() => go("engine")}>
                Voice engine
              </button>
            }
          />
          <Step
            done={s.permissions.inputMonitoring || s.hotkeyActive}
            title={`Let Viary notice the ${s.hotkeyName} key`}
            text="Input Monitoring. Viary only watches the dictation key."
            action={
              <button type="button" className={btn} onClick={() => api.requestPermission("inputMonitoring")}>
                Allow…
              </button>
            }
          />
          <Step
            done={s.permissions.accessibility}
            title="Let Viary type into apps"
            text="Accessibility, to paste the text and see if a text field has focus."
            action={
              <button type="button" className={btn} onClick={() => api.requestPermission("accessibility")}>
                Allow…
              </button>
            }
          />
        </section>
      )}

      <div className="grid grid-cols-3 gap-4">
        <div className="flex flex-col gap-2.5 rounded-[14px] border border-line bg-white px-[22px] py-5">
          <span className="text-[13px] text-muted">Words this week</span>
          <span className="font-serif text-[40px] leading-none">{words.toLocaleString()}</span>
          <span className="text-[13px] text-muted">
            Across {week.length} dictation{week.length === 1 ? "" : "s"}
          </span>
        </div>
        <div className="flex flex-col gap-2.5 rounded-[14px] border border-line bg-white px-[22px] py-5">
          <span className="text-[13px] text-muted">Speaking speed</span>
          <span className="font-serif text-[40px] leading-none">
            {wpm ?? "—"} <span className="text-xl text-muted">wpm</span>
          </span>
          <span className="text-[13px] text-muted">Words over the time you held the key</span>
        </div>
        <div className="flex flex-col gap-2.5 rounded-[14px] border border-line bg-white px-[22px] py-5">
          <span className="text-[13px] text-muted">Voice engine</span>
          <span className="truncate font-serif text-[28px] leading-tight">{s.engine.active?.name ?? "None yet"}</span>
          <span className="text-[13px] text-muted">
            {s.engine.active ? (s.engine.active.onDevice ? "On this Mac" : `Cloud · ${s.engine.active.kind}`) : "Choose one to start"}
          </span>
        </div>
      </div>

      <div className="flex flex-col overflow-hidden rounded-[14px] border border-line bg-white">
        <div className="flex items-center justify-between border-b border-sand px-[22px] py-4">
          <h2 className="m-0 text-[15px] font-semibold">Recent</h2>
          <button type="button" className="text-[13px] text-blue" onClick={() => go("history")}>
            All history
          </button>
        </div>
        {items.length === 0 && (
          <p className="m-0 px-[22px] py-6 text-sm text-muted">Your dictations will show up here.</p>
        )}
        {items.slice(0, 5).map((r) => (
          <div key={r.id} className="flex items-center gap-4 border-b border-hair px-[22px] py-3.5 last:border-b-0">
            <span className="w-16 font-mono text-xs text-faint">
              {new Date(r.createdAt).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })}
            </span>
            <span className="w-[90px] truncate text-[13px] font-medium">{r.app || "—"}</span>
            <span className="w-0 grow truncate text-sm text-body">
              {r.status === "failed" ? <span className="text-rust">Failed · audio kept</span> : r.text}
            </span>
            <span className="text-xs text-faint">{r.words} words</span>
          </div>
        ))}
      </div>
    </main>
  );
}
