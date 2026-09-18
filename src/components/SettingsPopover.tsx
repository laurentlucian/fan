import { useEffect, useRef, useState } from "react";
import type { AppSettings } from "../types";

const ROWS: [keyof AppSettings, string][] = [
  ["autostart", "Launch at login"],
  ["start_on_launch", "Start on launch"],
  ["close_to_tray", "Close to tray"],
];

export default function SettingsPopover({
  settings,
  onChange,
}: {
  settings: AppSettings;
  onChange: (patch: Partial<AppSettings>) => void;
}) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <div className="settings" ref={box}>
      <button
        className="gear"
        data-open={open || undefined}
        title="Settings"
        onClick={() => setOpen((v) => !v)}
      >
        <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
          <circle
            cx="8"
            cy="8"
            r="2.2"
            stroke="currentColor"
            strokeWidth="1.3"
            fill="none"
          />
          <path
            d="M8 1.6v1.8M8 12.6v1.8M1.6 8h1.8M12.6 8h1.8M3.5 3.5l1.3 1.3M11.2 11.2l1.3 1.3M12.5 3.5l-1.3 1.3M4.8 11.2l-1.3 1.3"
            stroke="currentColor"
            strokeWidth="1.3"
            strokeLinecap="round"
          />
        </svg>
      </button>

      {open && (
        <div className="popover">
          {ROWS.map(([key, label]) => (
            <label key={key} className="toggle">
              <input
                type="checkbox"
                checked={settings[key]}
                onChange={(e) => onChange({ [key]: e.currentTarget.checked })}
              />
              <span>{label}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}
