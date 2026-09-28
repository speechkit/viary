// Settings: the dictation key and the permissions Viary needs.

import { Icon, Kbd } from "../components/Icons";
import { api, type Hotkey, type Snapshot } from "../lib/ipc";

const btn = "inline-flex h-8 shrink-0 items-center whitespace-nowrap gap-2 rounded-lg border border-edge bg-white px-3 text-[13px] font-medium text-ink";

const KEYS: [Hotkey, string, string][] = [
  ["fn", "fn", "The globe key. Set System Settings › Keyboard › “Press 🌐 key to” to “Do Nothing”."],
  ["rightOption", "right ⌥", "The Option key right of the space bar."],
  ["rightCommand", "right ⌘", "The Command key right of the space bar."],
];

function Permission({
  granted,
  title,
  text,
  kind,
}: {
  granted: boolean;
  title: string;
  text: string;
  kind: "accessibility" | "inputMonitoring" | "microphone";
}) {
  return (
    <div className="flex items-center gap-3.5 border-b border-hair px-[18px] py-3 last:border-b-0">
      <span className={`size-2 shrink-0 rounded-full ${granted ? "bg-teal" : "bg-amber"}`} />
      <div className="flex min-w-0 grow flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-[13px] leading-[1.45] text-muted">{text}</span>
      </div>
      {granted ? (
        <span className="text-[13px] text-teal-ink">Allowed</span>
      ) : (
        <button type="button" className={btn} onClick={() => api.requestPermission(kind)}>
          Open System Settings
        </button>
      )}
    </div>
  );
}

export function SettingsPage({ s }: { s: Snapshot }) {
  return (
    <main className="flex max-w-[900px] flex-col gap-[18px] px-12 pt-9 pb-12">
      <h1 className="m-0 font-serif text-[40px] font-normal">Settings</h1>

      <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">Dictation key</h2>
      <div role="radiogroup" aria-label="Dictation key" className="grid grid-cols-3 gap-3.5">
        {KEYS.map(([key, label, text]) => {
          const on = s.settings.hotkey === key;
          return (
            <button
              key={key}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => api.setPreferences({ hotkey: key })}
              className={
                "flex flex-col gap-2.5 rounded-[14px] bg-white px-[18px] py-4 text-left " +
                (on ? "border-2 border-ink" : "border border-line")
              }
            >
              <span className="flex w-full items-center justify-between">
                <Kbd large>{label}</Kbd>
                <span className="size-[18px] rounded-full" style={{ border: on ? "5px solid #1C1B18" : "1.5px solid #CFC8B8" }} />
              </span>
              <span className="text-[13px] leading-[1.45] text-muted">{text}</span>
            </button>
          );
        })}
      </div>
      <span className="text-[13px] text-muted">Hold the key while you speak and release it to insert. A quick tap does nothing.</span>

      <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">Permissions</h2>
      <div className="overflow-hidden rounded-[14px] border border-line bg-white">
        <Permission
          granted={s.permissions.inputMonitoring && s.hotkeyActive}
          kind="inputMonitoring"
          title="Input Monitoring"
          text={`To notice when you hold ${s.hotkeyName}. Viary does not record other keys.`}
        />
        <Permission
          granted={s.permissions.accessibility}
          kind="accessibility"
          title="Accessibility"
          text="To paste into the app you are using, and to check that a text field has focus."
        />
        <div className="flex items-center gap-3.5 px-[18px] py-3">
          <Icon name="mic" />
          <div className="flex min-w-0 grow flex-col gap-0.5">
            <span className="text-sm font-medium">Microphone</span>
            <span className="text-[13px] leading-[1.45] text-muted">macOS asks the first time you dictate.</span>
          </div>
          <button type="button" className={btn} onClick={() => api.requestPermission("microphone")}>
            Open System Settings
          </button>
        </div>
      </div>
      <span className="text-[13px] text-muted">
        After allowing a permission, macOS may ask you to quit and reopen Viary.
      </span>
    </main>
  );
}
