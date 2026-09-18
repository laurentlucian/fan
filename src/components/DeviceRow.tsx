import { useState } from "react";
import Meter from "./Meter";
import type { DeviceInfo, OutputConfig, OutputStats } from "../types";

const clamp = (n: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, n));

export default function DeviceRow({
  device,
  output,
  level,
  stats,
  running,
  onToggle,
  onChange,
}: {
  device: DeviceInfo;
  output?: OutputConfig;
  level: number;
  stats?: OutputStats;
  running: boolean;
  onToggle: (on: boolean) => void;
  onChange: (patch: Partial<OutputConfig>) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const on = !!output;

  return (
    <div className="row" data-on={on || undefined}>
      <label className="pick">
        <input
          type="checkbox"
          checked={on}
          onChange={(e) => onToggle(e.currentTarget.checked)}
        />
        <span className="name">{device.name}</span>
        {device.kind !== "other" && <span className="kind">{device.kind}</span>}
        {on && running && stats && !stats.connected && (
          <span className="offline">offline</span>
        )}
      </label>

      {output && (
        <div className="ctl">
          <Meter level={level} />

          <input
            className="offset"
            type="number"
            min={-200}
            max={200}
            step={5}
            title="Offset"
            value={draft ?? String(output.offset_ms)}
            onChange={(e) => {
              const raw = e.currentTarget.value;
              setDraft(raw);
              const n = Number(raw);
              if (raw.trim() !== "" && Number.isFinite(n)) {
                onChange({ offset_ms: clamp(Math.round(n), -200, 200) });
              }
            }}
            onBlur={() => setDraft(null)}
          />
          <span className="unit">ms</span>

          <input
            className="gain"
            type="range"
            min={-40}
            max={12}
            step={0.5}
            title="Gain"
            value={output.gain_db}
            onChange={(e) => onChange({ gain_db: Number(e.currentTarget.value) })}
          />
          <span className="db">
            {output.gain_db === 0
              ? ""
              : `${output.gain_db > 0 ? "+" : ""}${output.gain_db}`}
          </span>

          <button
            className="mute"
            data-muted={output.muted || undefined}
            title={output.muted ? "Unmute" : "Mute"}
            onClick={() => onChange({ muted: !output.muted })}
          >
            <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
              <path
                d="M8 2.5 4.6 5.4H2.2v5.2h2.4L8 13.5z"
                fill="currentColor"
              />
              {output.muted ? (
                <path
                  d="M10.6 6.2l3.2 3.6M13.8 6.2l-3.2 3.6"
                  stroke="currentColor"
                  strokeWidth="1.3"
                  strokeLinecap="round"
                  fill="none"
                />
              ) : (
                <path
                  d="M10.8 5.9a3 3 0 0 1 0 4.2M12.7 4.2a5.6 5.6 0 0 1 0 7.6"
                  stroke="currentColor"
                  strokeWidth="1.2"
                  strokeLinecap="round"
                  fill="none"
                />
              )}
            </svg>
          </button>
        </div>
      )}
    </div>
  );
}
