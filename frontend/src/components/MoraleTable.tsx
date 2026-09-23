// Morale over the horizon, one sparkline per operator, grouped by the room
// they start in. One series per chart, so no legend: the row names it. The
// table's numbers (lowest morale, hours working, resting, exhausted) are
// the non-hover view of the same data.

import { useState } from "react";
import type { AssignmentMap, BaseConfig, SimResult } from "../api";
import { useGameData } from "../data";
import { fmt, roomName } from "../format";

const W = 240;
const H = 36;
const PAD = 4;

interface Sample {
  hour: number;
  mood: number;
}

function Sparkline({ samples, horizon, max, label }: { samples: Sample[]; horizon: number; max: number; label: string }) {
  const [at, setAt] = useState<number | null>(null);
  const x = (hour: number) => PAD + (hour / Math.max(horizon, 1e-9)) * (W - 2 * PAD);
  const y = (mood: number) => PAD + (1 - mood / max) * (H - 2 * PAD);
  const path = samples.map((s, i) => `${i === 0 ? "M" : "L"}${x(s.hour).toFixed(1)},${y(s.mood).toFixed(1)}`).join("");
  const firstEmpty = samples.find((s) => s.mood <= 1e-9);
  const hovered = at === null ? undefined : samples[at];

  const nearest = (clientX: number, rect: DOMRect) => {
    const hour = ((clientX - rect.left) / rect.width) * W;
    let best = 0;
    for (let i = 1; i < samples.length; i++) {
      const s = samples[i];
      const b = samples[best];
      if (s && b && Math.abs(x(s.hour) - hour) < Math.abs(x(b.hour) - hour)) best = i;
    }
    return best;
  };

  return (
    <span className="spark">
      <svg
        viewBox={`0 0 ${W} ${H}`}
        width={W}
        height={H}
        role="img"
        aria-label={label}
        tabIndex={0}
        onPointerMove={(e) => setAt(nearest(e.clientX, e.currentTarget.getBoundingClientRect()))}
        onPointerLeave={() => setAt(null)}
        onFocus={() => setAt(samples.length - 1)}
        onBlur={() => setAt(null)}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") setAt(Math.max(0, (at ?? samples.length - 1) - 1));
          if (e.key === "ArrowRight") setAt(Math.min(samples.length - 1, (at ?? 0) + 1));
        }}
      >
        <line className="spark-axis" x1={PAD} x2={W - PAD} y1={y(0)} y2={y(0)} />
        <path className="spark-line" d={path} />
        {firstEmpty && <circle className="spark-empty" cx={x(firstEmpty.hour)} cy={y(0)} r={4} />}
        {hovered && (
          <>
            <line className="spark-cross" x1={x(hovered.hour)} x2={x(hovered.hour)} y1={0} y2={H} />
            <circle className="spark-dot" cx={x(hovered.hour)} cy={y(hovered.mood)} r={4} />
          </>
        )}
      </svg>
      {hovered && (
        <span className="spark-tip" style={{ left: `${(x(hovered.hour) / W) * 100}%` }}>
          <strong>{fmt(hovered.mood, 1)}</strong> <span className="muted">at {fmt(hovered.hour, 0)} h</span>
        </span>
      )}
    </span>
  );
}

export function MoraleTable({ sim, base, assignment }: { sim: SimResult; base: BaseConfig; assignment: AssignmentMap }) {
  const { name } = useGameData();
  const byOp = new Map<string, Sample[]>();
  for (const s of sim.trajectory) {
    const list = byOp.get(s.operator) ?? [];
    list.push({ hour: s.hour, mood: s.mood });
    byOp.set(s.operator, list);
  }
  for (const list of byOp.values()) list.sort((a, b) => a.hour - b.hour);
  const reports = new Map(sim.operators.map((o) => [o.id, o]));

  const groups: { title: string; ops: string[] }[] = [];
  const seen = new Set<string>();
  for (const room of base.rooms) {
    const ops = (assignment[room.id] ?? []).filter((o): o is string => o !== null);
    ops.forEach((o) => seen.add(o));
    if (ops.length) groups.push({ title: `${roomName(room.kind)} ${room.id}`, ops });
  }
  const bench = sim.operators.map((o) => o.id).filter((id) => !seen.has(id));
  if (bench.length) groups.push({ title: "Relief (starts off the base)", ops: bench });

  return (
    <div className="table-scroll">
      <table className="morale">
        <thead>
          <tr>
            <th>Operator</th>
            <th>
              Morale, 0 to 24, over {fmt(sim.horizon_hours)} h
            </th>
            <th className="num">Lowest</th>
            <th className="num">Working</th>
            <th className="num">Resting</th>
            <th className="num">Exhausted</th>
          </tr>
        </thead>
        {groups.map((g) => (
          <tbody key={g.title}>
            <tr className="group">
              <th colSpan={6}>{g.title}</th>
            </tr>
            {g.ops.map((op) => {
              const r = reports.get(op);
              const samples = byOp.get(op) ?? [];
              const exhausted = (r?.hours_exhausted ?? 0) > 0;
              return (
                <tr key={op}>
                  <td>{name(op)}</td>
                  <td>
                    {samples.length > 1 ? (
                      <Sparkline
                        samples={samples}
                        horizon={sim.horizon_hours}
                        max={24}
                        label={`${name(op)}: morale from ${fmt(samples[0]?.mood ?? 0, 1)} to ${fmt(samples.at(-1)?.mood ?? 0, 1)}, lowest ${fmt(r?.min_mood ?? 0, 1)}`}
                      />
                    ) : (
                      <span className="muted small">idle</span>
                    )}
                  </td>
                  <td className="num">
                    {fmt(r?.min_mood ?? 0, 1)}
                    {exhausted && (
                      <span className="status-critical small">
                        {" "}
                        <span aria-hidden="true">●</span> exhausted
                      </span>
                    )}
                  </td>
                  <td className="num">{fmt(r?.hours_working ?? 0)} h</td>
                  <td className="num">{fmt(r?.hours_resting ?? 0)} h</td>
                  <td className="num">{fmt(r?.hours_exhausted ?? 0)} h</td>
                </tr>
              );
            })}
          </tbody>
        ))}
      </table>
    </div>
  );
}
