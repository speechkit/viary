// Form controls shared by the main window's pages, drawn as in the design.

import { useEffect, useRef, useState } from "react";
import { Icon } from "./Icons";

export const btn =
  "inline-flex h-8 shrink-0 items-center whitespace-nowrap gap-2 rounded-lg border border-edge bg-white px-3 text-[13px] font-medium text-ink disabled:opacity-50";
export const btnPrimary =
  "inline-flex h-8 shrink-0 items-center whitespace-nowrap gap-2 rounded-lg border border-ink bg-ink px-3 text-[13px] font-medium text-white disabled:opacity-40";
export const input =
  "h-[34px] rounded-lg border border-edge bg-white px-2.5 text-[13px] text-ink outline-none placeholder:text-faint focus:border-ink";

/**
 * The design's dropdown: a white 34 px button, and a menu card drawn in the
 * page (the native select opens the system menu, which ignores the design).
 */
export function Select<T extends string | number>({
  label,
  value,
  options,
  onChange,
  disabled = false,
}: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
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
        disabled={disabled}
        onClick={() => (open ? setOpen(false) : show())}
        className={
          "flex h-[34px] max-w-[300px] items-center gap-2 rounded-lg border bg-white pr-2.5 pl-3 text-[13px] font-medium text-ink disabled:opacity-50 " +
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

export function H2({ children }: { children: React.ReactNode }) {
  return <h2 className="m-0 text-xs font-semibold tracking-[.04em] text-faint uppercase">{children}</h2>;
}

export function Tag({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex h-[22px] items-center rounded-full bg-paper px-2 text-[11px] font-medium text-muted">
      {children}
    </span>
  );
}

export function Seg<T extends string>({
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

/** An on/off switch: 40×24 on pages, 36×22 in the menu bar popover. */
export function Switch({
  on,
  label,
  onChange,
  small = false,
  disabled = false,
}: {
  on: boolean;
  label: string;
  onChange: (on: boolean) => void;
  small?: boolean;
  disabled?: boolean;
}) {
  const [w, h, knob] = small ? [36, 22, 16] : [40, 24, 18];
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={"relative shrink-0 rounded-full p-0 transition-colors disabled:opacity-40 " + (on ? "bg-ink" : "bg-stone")}
      style={{ width: w, height: h }}
    >
      <span
        aria-hidden="true"
        className="absolute top-[3px] rounded-full bg-white shadow-[0_1px_2px_rgba(0,0,0,.2)] transition-[left]"
        style={{ width: knob, height: knob, left: on ? w - knob - 3 : 3 }}
      />
    </button>
  );
}

export function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="grid grid-cols-[96px_1fr] items-center gap-3 text-[13px]">
      <span className="text-muted">{label}</span>
      {children}
    </label>
  );
}

/** A dismissible error line at the top of a page. */
export function ErrorBanner({ error, onDismiss }: { error: string; onDismiss: () => void }) {
  if (!error) return null;
  return (
    <div role="alert" className="flex items-start gap-2 rounded-[10px] bg-rust-wash px-3 py-2.5 text-[13px] text-rust">
      <Icon name="warning" />
      <span className="grow">{error}</span>
      <button type="button" aria-label="Dismiss" onClick={onDismiss}>
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}

/**
 * A form's unsaved copy of `saved`. It follows `saved` only when the stored
 * value really changes: every refresh brings a new object, and resetting
 * on identity would throw away what the user is typing.
 */
export function useDraft<T>(saved: T): [T, (value: T) => void, boolean] {
  const key = JSON.stringify(saved);
  const [draft, setDraft] = useState<T>(saved);
  useEffect(() => setDraft(JSON.parse(key) as T), [key]);
  return [draft, setDraft, JSON.stringify(draft) !== key];
}
