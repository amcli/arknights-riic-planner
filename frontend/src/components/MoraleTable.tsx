// Morale over the horizon, one sparkline per operator, grouped by the room
// they start in. One series per chart, so no legend: the row names it. The
// compact view packs the groups into columns with each operator's lowest
// morale; the table view is the full non-hover twin (lowest morale, hours
// working, resting, exhausted).

import { useState } from "react";
import type { AssignmentMap, BaseConfig, SimResult } from "../api";
import { useGameData } from "../data";
import { fmt, roomName } from "../format";

const PAD = 4;

interface Sample {
  hour: number;
  mood: number;
}

function Sparkline({
  samples,
  horizon,
  max,
  label,
  width,
  height,
}: {
  samples: Sample[];
  horizon: number;
  max: number;
  label: string;
  width: number;
  height: number;
}) {
  const [at, setAt] = useState<number | null>(null);
  const x = (hour: number) => PAD + (hour / Math.max(horizon, 1e-9)) * (width - 2 * PAD);
  const y = (mood: number) => PAD + (1 - mood / max) * (height - 2 * PAD);
  const path = samples.map((s, i) => `${i === 0 ? "M" : "L"}${x(s.hour).toFixed(1)},${y(s.mood).toFixed(1)}`).join("");
  const firstEmpty = samples.find((s) => s.mood <= 1e-9);
  const hovered = at === null ? undefined : samples[at];

  const nearest = (clientX: number, rect: DOMRect) => {
    const px = ((clientX - rect.left) / rect.width) * width;
    let best = 0;
    for (let i = 1; i < samples.length; i++) {
      const s = samples[i];
      const b = samples[best];
      if (s && b && Math.abs(x(s.hour) - px) < Math.abs(x(b.hour) - px)) best = i;
    }
    return best;
  };

  return (
    <span className="spark">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        width={width}
        height={height}
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
        <line className="spark-axis" x1={PAD} x2={width - PAD} y1={y(0)} y2={y(0)} />
        <path className="spark-line" d={path} />
        {firstEmpty && <circle className="spark-empty" cx={x(firstEmpty.hour)} cy={y(0)} r={4} />}
        {hovered && (
          <>
            <line className="spark-cross" x1={x(hovered.hour)} x2={x(hovered.hour)} y1={0} y2={height} />
            <circle className="spark-dot" cx={x(hovered.hour)} cy={y(hovered.mood)} r={4} />
          </>
        )}
      </svg>
      {hovered && (
        <span className="spark-tip" style={{ left: `${(x(hovered.hour) / width) * 100}%` }}>
          <strong>{fmt(hovered.mood, 1)}</strong> <span className="muted">at {fmt(hovered.hour, 0)} h</span>
        </span>
      )}
    </span>
  );
}

export type MoraleView = "chart" | "table";

export function MoraleTable({
  sim,
  base,
  assignment,
  view = "chart",
}: {
  sim: SimResult;
  base: BaseConfig;
  assignment: AssignmentMap;
  view?: MoraleView;
}) {
  const { name } = useGameData();
  const byOp = new Map<string, Sample[]>();
  for (const s of sim.trajectory) {
    const list = byOp.get(s.operator) ?? [];
    list.push({ hour: s.hour, mood: s.mood });
    byOp.set(s.operator, list);
  }
  for (const list of byOp.values()) list.sort((a, b) => a.hour - b.hour);
  const reports = new Map(sim.operators.map((o) => [o.id, o]));

  const groups: { key: string; title: string; id?: string; ops: string[] }[] = [];
  const seen = new Set<string>();
  for (const room of base.rooms) {
    const ops = (assignment[room.id] ?? []).filter((o): o is string => o !== null);
    ops.forEach((o) => seen.add(o));
    if (ops.length) groups.push({ key: room.id, title: roomName(room.kind), id: room.id, ops });
  }
  // Relief that never came on: often most of the roster, every one a flat
  // line at full morale, so they are named in one line rather than charted.
  const unused = (id: string) => {
    const r = reports.get(id);
    return !r || (r.hours_working <= 0 && r.hours_resting <= 0 && r.hours_exhausted <= 0);
  };
  const bench = sim.operators.map((o) => o.id).filter((id) => !seen.has(id));
  const relief = bench.filter((id) => !unused(id));
  const offBase = bench.filter(unused);
  if (relief.length) groups.push({ key: "relief", title: "Relief (starts off the base)", ops: relief });
  const offBaseNote = offBase.length > 0 && (
    <details className="hint off-base">
      <summary>
        {fmt(offBase.length)} more {offBase.length === 1 ? "operator" : "operators"} stayed off the base throughout, at full
        morale
      </summary>
      <p>{offBase.map(name).join(", ")}</p>
    </details>
  );

  const describe = (op: string, samples: Sample[]) => {
    const r = reports.get(op);
    return `${name(op)}: morale from ${fmt(samples[0]?.mood ?? 0, 1)} to ${fmt(samples.at(-1)?.mood ?? 0, 1)}, lowest ${fmt(r?.min_mood ?? 0, 1)}`;
  };

  if (view === "chart") {
    const anyExhausted = sim.operators.some((o) => o.hours_exhausted > 0);
    const rowClass = anyExhausted ? "morale-row has-exhausted" : "morale-row";
    return (
      <>
        <div className="morale-cols">
          {groups.map((g) => (
            // Short groups stay in one column; long ones may flow on.
            <div key={g.key} className={g.ops.length > 8 ? "morale-group long" : "morale-group"}>
              <div className={anyExhausted ? "morale-group-head has-exhausted" : "morale-group-head"}>
                <span className="ellipsis">
                  {g.title} {g.id && <span className="id">{g.id}</span>}
                </span>
                <span className="tiny muted num">lowest</span>
                {anyExhausted && <span className="tiny muted num">exhausted</span>}
              </div>
              {g.ops.map((op) => {
                const r = reports.get(op);
                const samples = byOp.get(op) ?? [];
                const exhausted = r?.hours_exhausted ?? 0;
                return (
                  <div key={op} className={rowClass}>
                    <span className="morale-name" title={name(op)}>
                      {name(op)}
                    </span>
                    {samples.length > 1 ? (
                      <Sparkline
                        samples={samples}
                        horizon={sim.horizon_hours}
                        max={24}
                        width={104}
                        height={22}
                        label={describe(op, samples)}
                      />
                    ) : (
                      <span className="muted tiny">idle</span>
                    )}
                    <span className="num">{fmt(r?.min_mood ?? 0, 1)}</span>
                    {anyExhausted && (
                      <span className="exhausted">
                        {exhausted > 0 && (
                          <>
                            <span className="status-dot" aria-hidden="true" />
                            {fmt(exhausted, 1)} h
                          </>
                        )}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
        {offBaseNote}
      </>
    );
  }

  return (
    <div className="table-wrap">
      <table className="morale dense">
        <thead>
          <tr>
            <th>Operator</th>
            <th>Morale, 0 to 24, over {fmt(sim.horizon_hours)} h</th>
            <th className="num">Lowest</th>
            <th className="num">Working</th>
            <th className="num">Resting</th>
            <th className="num">Exhausted</th>
          </tr>
        </thead>
        {groups.map((g) => (
          <tbody key={g.key}>
            <tr className="group">
              <th colSpan={6}>
                {g.title} {g.id && <span className="id">{g.id}</span>}
              </th>
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
                        width={200}
                        height={26}
                        label={describe(op, samples)}
                      />
                    ) : (
                      <span className="muted small">idle</span>
                    )}
                  </td>
                  <td className="num">
                    {fmt(r?.min_mood ?? 0, 1)}
                    {exhausted && (
                      <span className="exhausted">
                        {" "}
                        <span className="status-dot" aria-hidden="true" /> exhausted
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
      {offBaseNote}
    </div>
  );
}
