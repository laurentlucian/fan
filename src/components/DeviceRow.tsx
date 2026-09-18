import Meter from "./Meter";
import type { DeviceInfo, OutputConfig, OutputStats } from "../types";

const GAIN_MIN = -40;
const GAIN_MAX = 12;
const GAIN_STEP = 0.5;
const OFFSET_MIN = -200;
const OFFSET_MAX = 200;
const OFFSET_STEP = 1;

const clamp = (n: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, n));

const roundDb = (n: number) =>
  clamp(Math.round(n * 10) / 10, GAIN_MIN, GAIN_MAX);

const fmtDb = (n: number) => {
  const v = roundDb(n);
  return Number.isInteger(v) ? String(v) : v.toFixed(1);
};

function Step({
  value,
  min,
  max,
  onNudge,
  onReset,
}: {
  value: string;
  min: boolean;
  max: boolean;
  onNudge: (dir: -1 | 1) => void;
  onReset: () => void;
}) {
  return (
    <div className="step">
      <button
        className="nudge"
        disabled={min}
        aria-label="Down"
        onClick={() => onNudge(-1)}
      >
        −
      </button>
      <span className="step-val" title="Reset" onDoubleClick={onReset}>
        {value}
      </span>
      <button
        className="nudge"
        disabled={max}
        aria-label="Up"
        onClick={() => onNudge(1)}
      >
        +
      </button>
    </div>
  );
}

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
  const on = !!output;
  const gain = output ? roundDb(output.gain_db) : 0;

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

          <Step
            value={`${output.offset_ms} ms`}
            min={output.offset_ms <= OFFSET_MIN}
            max={output.offset_ms >= OFFSET_MAX}
            onNudge={(dir) =>
              onChange({
                offset_ms: clamp(
                  output.offset_ms + dir * OFFSET_STEP,
                  OFFSET_MIN,
                  OFFSET_MAX,
                ),
              })
            }
            onReset={() => onChange({ offset_ms: 0 })}
          />
          <Step
            value={`${fmtDb(gain)} dB`}
            min={gain <= GAIN_MIN}
            max={gain >= GAIN_MAX}
            onNudge={(dir) =>
              onChange({ gain_db: roundDb(gain + dir * GAIN_STEP) })
            }
            onReset={() => onChange({ gain_db: 0 })}
          />

          <button
            className="mute"
            data-muted={output.muted || undefined}
            title={output.muted ? "Unmute" : "Mute"}
            onClick={() => onChange({ muted: !output.muted })}
          >
            <svg width="20" height="20" viewBox="0 0 16 16" aria-hidden="true">
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
