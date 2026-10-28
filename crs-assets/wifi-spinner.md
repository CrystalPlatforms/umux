import React from "react";

export function Cascade({
  size = 20,
  color,
  duration = 1500,
  cap = "round",
  playState = "running",
  className,
}) {
  const SLOTS = 24;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      role="presentation"
      className={className}
      stroke={color ?? "currentColor"}
      strokeLinecap={cap === "flat" ? "butt" : "round"}
      strokeWidth="2"
      style={{ transform: "rotate(180deg)" }}
    >
      {[10.5, 7, 3.5].map((r, i) => {
        const circumference = 2 * Math.PI * r;
        return (
          <circle
            key={r}
            cx="12"
            cy="12"
            r={r}
            strokeDasharray={`${circumference / 4} ${(circumference * 3) / 4}`}
            style={{
              transformOrigin: "center",
              animation: `ld-cascade-turn ${duration}ms cubic-bezier(0.68, -0.75, 0.265, 1.75) ${(-duration * i) / SLOTS}ms infinite`,
              animationPlayState: playState,
            }}
          />
        );
      })}
    </svg>
  );
}

function PausedCascade({ size = 20 }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      role="presentation"
      stroke="white"
      strokeWidth="2"
      strokeLinecap="round"
    >
      {[10.5, 7, 3.5].map((r) => {
        return (
          <path
            key={r}
            d={describeArc(12, 12, r, -135, -45)}
            stroke="white"
            fill="none"
          />
        );
      })}
    </svg>
  );
}

function polarToCartesian(cx, cy, r, deg) {
  const rad = ((deg - 0) * Math.PI) / 180;
  return { x: cx + r * Math.cos(rad), y: cy + r * Math.sin(rad) };
}

function describeArc(cx, cy, r, start, end) {
  const s = polarToCartesian(cx, cy, r, end);
  const e = polarToCartesian(cx, cy, r, start);
  const largeArc = end - start <= 180 ? 0 : 1;
  return `M ${s.x} ${s.y} A ${r} ${r} 0 ${largeArc} 0 ${e.x} ${e.y}`;
}

export default function App() {
  return (
    <div className="min-h-screen bg-black flex flex-col items-center justify-center gap-14 p-10">
      <style>{`
        @keyframes ld-cascade-turn {
          to { transform: rotate(360deg); }
        }
      `}</style>
      <div className="text-center">
        <h1 className="text-2xl font-semibold tracking-tight text-white">CRS-OS WiFi connecting loader</h1>
      </div>

      <div className="flex items-center gap-16 text-white">
        <div className="flex flex-col items-center gap-3">
          <Cascade size={16} />
          <span className="text-xs text-zinc-600">16px</span>
        </div>
        <div className="flex flex-col items-center gap-3">
          <Cascade size={48} />
          <span className="text-xs text-zinc-600">48px</span>
        </div>
        <div className="flex flex-col items-center gap-3">
          <Cascade size={72} />
          <span className="text-xs text-zinc-600">72px</span>
        </div>
      </div>

      <div className="flex items-center gap-16">
        <div className="flex flex-col items-center gap-3">
          <PausedCascade size={16} />
          <span className="text-xs text-zinc-600">16px</span>
        </div>
        <div className="flex flex-col items-center gap-3">
          <PausedCascade size={48} />
          <span className="text-xs text-zinc-600">48px</span>
        </div>
        <div className="flex flex-col items-center gap-3">
          <PausedCascade size={72} />
          <span className="text-xs text-zinc-600">72px</span>
        </div>
      </div>
    </div>
  );
}
