import { useState } from "react";
import {
  api,
  type RoomStats,
  type RoomType,
  type SimRequest,
  type SimResult,
  type Snapshot,
  type Tagged,
} from "./api";

// A small valid base: Control Center with Team Rainbow, one Power Plant,
// one gold Factory, the Texas/Lappland/Exusiai Trading Post and a Dormitory.
// Larger examples live in examples/requests/ in the repository.
export const DEFAULT_REQUEST = {
  base: {
    rooms: [
      { id: "cc", kind: "CONTROL", level: 5 },
      { id: "p1", kind: "POWER", level: 3 },
      { id: "f1", kind: "MANUFACTURE", level: 3, settings: { formula: "4" } },
      { id: "t1", kind: "TRADING", level: 3 },
      { id: "d1", kind: "DORMITORY", level: 5, settings: { ambience: 5000 } },
    ],
  },
  assignment: {
    cc: ["char_456_ash", "char_457_blitz", "char_458_rfrost", "char_459_tachak", null],
    p1: ["char_253_greyy"],
    f1: ["char_190_clour", null, null],
    t1: ["char_102_texas", "char_140_whitew", "char_103_angel"],
    d1: [null, null, null, null, null],
  },
  roster: Object.fromEntries(
    [
      "char_456_ash",
      "char_457_blitz",
      "char_458_rfrost",
      "char_459_tachak",
      "char_253_greyy",
      "char_190_clour",
      "char_102_texas",
      "char_140_whitew",
      "char_103_angel",
    ].map((id) => [id, { promotion: { phase: "PHASE_2", level: 90 } }]),
  ),
  config: { horizon_hours: 24, tick_minutes: 5, initial_gold: 20 },
};

type Outcome =
  | { state: "idle" }
  | { state: "running" }
  | { state: "error"; message: string }
  | { state: "snapshot"; data: Snapshot }
  | { state: "result"; data: SimResult };

const fmt = (n: number, digits = 1) =>
  n.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits });

function mainStat(r: RoomStats): number | null {
  const byKind: Partial<Record<RoomType, number>> = {
    MANUFACTURE: r.productivity_pct,
    TRADING: r.order_efficiency_pct,
    POWER: r.drone_recovery_pct,
    MEETING: r.clue_speed_pct,
    HIRE: r.contact_speed_pct,
    TRAINING: r.training_speed_pct,
  };
  return byKind[r.kind] ?? null;
}

function describe(t: Tagged): string {
  const { kind, ...rest } = t;
  const fields = Object.entries(rest)
    .map(([k, v]) => `${k}: ${typeof v === "object" ? JSON.stringify(v) : String(v)}`)
    .join(", ");
  return fields ? `${kind} (${fields})` : kind;
}

export function Simulator({ names }: { names: Map<string, string> }) {
  const [text, setText] = useState(() => JSON.stringify(DEFAULT_REQUEST, null, 2));
  const [outcome, setOutcome] = useState<Outcome>({ state: "idle" });
  const name = (id: string) => names.get(id) ?? id;

  const run = async (mode: "evaluate" | "simulate") => {
    let request: SimRequest;
    try {
      request = JSON.parse(text) as SimRequest;
    } catch (err) {
      setOutcome({ state: "error", message: `Request is not valid JSON: ${String(err)}` });
      return;
    }
    setOutcome({ state: "running" });
    try {
      if (mode === "evaluate") {
        setOutcome({ state: "snapshot", data: await api.evaluate(request) });
      } else {
        setOutcome({ state: "result", data: await api.simulate(request) });
      }
    } catch (err) {
      setOutcome({ state: "error", message: err instanceof Error ? err.message : String(err) });
    }
  };

  return (
    <div className="card">
      <p className="muted">
        Edit the request, then evaluate the starting instant or simulate the whole horizon. The request format
        follows <code>examples/requests/</code> in the repository.
      </p>
      <textarea
        className="request"
        value={text}
        onChange={(e) => setText(e.target.value)}
        spellCheck={false}
        aria-label="Simulation request JSON"
        rows={14}
      />
      <div className="actions">
        <button type="button" onClick={() => void run("evaluate")} disabled={outcome.state === "running"}>
          Evaluate
        </button>
        <button type="button" onClick={() => void run("simulate")} disabled={outcome.state === "running"}>
          Simulate
        </button>
        <button type="button" className="secondary" onClick={() => setText(JSON.stringify(DEFAULT_REQUEST, null, 2))}>
          Reset
        </button>
      </div>
      {outcome.state === "running" && <p>Running…</p>}
      {outcome.state === "error" && <p className="error">{outcome.message}</p>}
      {outcome.state === "snapshot" && <SnapshotView snap={outcome.data} name={name} />}
      {outcome.state === "result" && <ResultView result={outcome.data} name={name} />}
    </div>
  );
}

function Warnings({ warnings }: { warnings: Tagged[] }) {
  if (warnings.length === 0) return <p className="muted">No warnings: every skill part involved was modelled.</p>;
  return (
    <>
      <h3>Warnings ({warnings.length})</h3>
      <ul className="warnings">
        {warnings.map((w, i) => (
          <li key={i} className="warn">
            {describe(w)}
          </li>
        ))}
      </ul>
    </>
  );
}

function SnapshotView({ snap, name }: { snap: Snapshot; name: (id: string) => string }) {
  return (
    <>
      <h3>Rooms at the starting instant</h3>
      <table>
        <thead>
          <tr>
            <th>Room</th>
            <th>Kind</th>
            <th>Operators</th>
            <th>Main stat</th>
          </tr>
        </thead>
        <tbody>
          {snap.rooms.map((r) => {
            const stat = mainStat(r);
            return (
              <tr key={r.id}>
                <td>{r.id}</td>
                <td>
                  {r.kind} L{r.level}
                </td>
                <td>{r.headcount}</td>
                <td>{stat === null ? "" : `${fmt(stat)}%`}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <h3>Morale per hour</h3>
      <table>
        <thead>
          <tr>
            <th>Operator</th>
            <th>Room</th>
            <th>Base</th>
            <th>Skills</th>
            <th>Total</th>
          </tr>
        </thead>
        <tbody>
          {snap.mood.map((m) => (
            <tr key={m.operator}>
              <td>
                {name(m.operator)}
                {m.idle && <span className="muted"> (idle)</span>}
                {m.exhausted && <span className="warn"> (exhausted)</span>}
              </td>
              <td>{m.room}</td>
              <td>{fmt(m.base, 2)}</td>
              <td>{fmt(m.skills, 2)}</td>
              <td>{fmt(m.total, 2)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>Skill contributions</h3>
      <table>
        <thead>
          <tr>
            <th>Room</th>
            <th>Operator</th>
            <th>Skill</th>
            <th>Stat</th>
            <th>Value</th>
          </tr>
        </thead>
        <tbody>
          {snap.contributions.map((c, i) => (
            <tr key={i}>
              <td>{c.room}</td>
              <td>{name(c.operator)}</td>
              <td>
                <code>{c.skill}</code>
              </td>
              <td>{c.stat}</td>
              <td>
                {fmt(c.value, 2)}
                {c.scaled_from !== undefined && <span className="muted"> (from {fmt(c.scaled_from, 2)})</span>}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <Warnings warnings={snap.warnings} />
    </>
  );
}

export function ResultView({ result, name }: { result: SimResult; name: (id: string) => string }) {
  const t = result.totals;
  const output = (r: SimResult["rooms"][number]) => {
    const parts: string[] = [];
    if (r.lmd > 0) parts.push(`${fmt(r.lmd, 0)} LMD`);
    if (r.orundum > 0) parts.push(`${fmt(r.orundum, 0)} Orundum`);
    for (const [item, n] of Object.entries(r.produced)) parts.push(`${fmt(n, 2)} × item ${item}`);
    if (r.drones > 0) parts.push(`${fmt(r.drones, 0)} drones`);
    if (r.contacts > 0) parts.push(`${fmt(r.contacts, 2)} contacts`);
    if (r.training_progress_hours > 0) {
      const done = r.training_completed_hour === null ? "" : `, done at ${fmt(r.training_completed_hour, 2)} h`;
      parts.push(`${fmt(r.training_progress_hours, 2)} training h${done}`);
    }
    if (r.hours_blocked > 0) parts.push(`blocked ${fmt(r.hours_blocked)} h`);
    return parts.join(" · ");
  };
  return (
    <>
      <h3>
        Totals over {fmt(result.horizon_hours, 0)} h
        <span className="muted">
          {" "}
          ({result.tick_minutes}-minute ticks, data {result.data.sha.slice(0, 12)})
        </span>
      </h3>
      <dl>
        <dt>LMD</dt>
        <dd>
          {fmt(t.lmd, 0)} from {fmt(t.orders_completed, 2)} orders
          {t.lmd_spent > 0 && <span className="muted"> · {fmt(t.lmd_spent, 0)} spent on Factory inputs</span>}
        </dd>
        <dt>EXP</dt>
        <dd>{fmt(t.exp, 0)}</dd>
        <dt>Pure Gold</dt>
        <dd>
          {fmt(t.gold_produced, 2)} made · {fmt(t.gold_consumed, 2)} sold · {fmt(t.gold_in_depot, 2)} left
        </dd>
        <dt>Orundum</dt>
        <dd>{fmt(t.orundum, 0)}</dd>
        <dt>Drones</dt>
        <dd>{fmt(t.drones, 0)}</dd>
        <dt>Contacts</dt>
        <dd>{fmt(t.contacts, 2)}</dd>
      </dl>
      <h3>Rooms</h3>
      <table>
        <thead>
          <tr>
            <th>Room</th>
            <th>Kind</th>
            <th>Average</th>
            <th>Output</th>
            <th>Operators at the end</th>
          </tr>
        </thead>
        <tbody>
          {result.rooms.map((r) => (
            <tr key={r.id}>
              <td>{r.id}</td>
              <td>
                {r.kind} L{r.level}
              </td>
              <td>{r.average_stat_pct === null ? "" : `${fmt(r.average_stat_pct)}%`}</td>
              <td>{output(r)}</td>
              <td>{r.operators.map(name).join(", ")}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>Operators</h3>
      <table>
        <thead>
          <tr>
            <th>Operator</th>
            <th>Morale</th>
            <th>Lowest</th>
            <th>Working</th>
            <th>Resting</th>
            <th>Exhausted</th>
            <th>Idle</th>
            <th>Benched</th>
          </tr>
        </thead>
        <tbody>
          {result.operators.map((o) => (
            <tr key={o.id}>
              <td>{o.name || o.id}</td>
              <td>
                {fmt(o.initial_mood)} → {fmt(o.final_mood)}
              </td>
              <td className={o.min_mood <= 0 ? "warn" : undefined}>{fmt(o.min_mood)}</td>
              <td>{fmt(o.hours_working)} h</td>
              <td>{fmt(o.hours_resting)} h</td>
              <td>{fmt(o.hours_exhausted)} h</td>
              <td>{fmt(o.hours_idle)} h</td>
              <td>{fmt(o.hours_benched)} h</td>
            </tr>
          ))}
        </tbody>
      </table>
      {result.events.length > 0 && (
        <>
          <h3>Events ({result.events.length})</h3>
          <ul className="events">
            {result.events.slice(0, 40).map((e, i) => (
              <li key={i}>
                <span className="muted">{fmt(e.hour, 2)} h</span> {describe(e.kind)}
              </li>
            ))}
            {result.events.length > 40 && <li className="muted">… {result.events.length - 40} more</li>}
          </ul>
        </>
      )}
      <Warnings warnings={result.warnings} />
    </>
  );
}
