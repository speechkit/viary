// A button with a drop-down menu. It closes on a click anywhere else, on
// Esc, and once an item is chosen. (WebKit does not focus a clicked
// button, so closing on blur would leave it open.)

import { useEffect, useRef, useState } from "react";

export interface MenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
}

export function Menu({
  label,
  items,
  className,
  children,
}: {
  /** The button's accessible name. */
  label: string;
  items: MenuItem[];
  className: string;
  /** The button's content. */
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={root} className="relative">
      <button type="button" aria-label={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)} className={className}>
        {children}
      </button>
      {open && (
        <div
          role="menu"
          className="absolute top-10 right-0 z-10 flex min-w-[180px] flex-col rounded-[10px] border border-line bg-white py-1 shadow-[0_8px_24px_rgba(28,27,24,0.16)]"
        >
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className={"px-3 py-2 text-left text-[13px] hover:bg-paper " + (item.danger ? "text-rust" : "")}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
