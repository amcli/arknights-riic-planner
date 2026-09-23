// Plan: choose a stored base and roster, say what counts and how far ahead
// to look, optionally pin operators where they are, and start a solve.

import { useEffect, useState } from "react";
import { api, type AssignmentMap, type BaseConfig, type DocumentMeta, type Roster, type Slot } from "../api";
import { useGameData } from "../data";
import { errorText, fmt, roomName } from "../format";
import { go, href } from "../router";
import { recall, remember } from "../storage";

interface Weight {
  key: string;
  label: string;
  help: string;
  value: number;
}

// The solver's defaults (`ak_solver::Objective`), cost-based where a cost
// exists.
const WEIGHTS: Weight[] = [
  { key: "lmd", label: "LMD", help: "per LMD from Trading Post orders", value: 1 },
  { key: "exp", label: "EXP", help: "per EXP of Battle Records made", value: 1 },
  { key: "orundum", label: "Orundum", help: "what 20 Orundum cost in Shards, LMD and Factory time", value: 200 },
  { key: "drones", label: "Drones", help: "three minutes of Factory time each", value: 20 },
  { key: "contacts", label: "Office contacts", help: "no LMD price; give one to value them", value: 0 },
  { key: "training_hours", label: "Training hours", help: "base hours of specialization progress", value: 0 },
  { key: "gold", label: "Unsold Pure Gold", help: "gold made but not sold within the horizon", value: 0 },
  { key: "exhausted_hour", label: "Exhausted hours (penalty)", help: "subtracted per operator-hour at zero morale", value: 0 },
];

const EFFORT = {
  quick: { label: "Quick", help: "about 20 s at most", solver: { iterations: 1500, restarts: 2, time_budget_ms: 20_000 } },
  standard: { label: "Standard", help: "about 90 s at most", solver: { iterations: 4000, restarts: 4, time_budget_ms: 90_000 } },
  thorough: { label: "Thorough", help: "up to 5 minutes", solver: { iterations: 12_000, restarts: 8, time_budget_ms: 300_000 } },
} as const;
type Effort = keyof typeof EFFORT;

interface Pin {
  operator: string;
  room: string;
}

/** An empty assignment shaped like the base, with the pins placed; and the pinned slots. */
function pinned(base: BaseConfig, pins: Pin[], capacity: (roomId: string) => number) {
  const initial: AssignmentMap = {};
  for (const r of base.rooms) initial[r.id] = Array<string | null>(capacity(r.id)).fill(null);
  const locked: Slot[] = [];
  const problems: string[] = [];
  for (const pin of pins) {
    const room = base.rooms.find((r) => r.id === pin.room);
    const slots = initial[pin.room];
    if (!room || !slots) {
      problems.push(`${pin.room} is not in this base`);
      continue;
    }
    // In a Training Room only the assistant's slot (0) can be pinned.
    const candidates = room.kind === "TRAINING" ? [0] : slots.map((_, i) => i);
    const index = candidates.find((i) => slots[i] === null);
    if (index === undefined) {
      problems.push(`${roomName(room.kind)} ${room.id} has no free slot for another pin`);
      continue;
    }
    slots[index] = pin.operator;
    locked.push({ room: pin.room, index });
  }
  return { initial, locked, problems };
}

export function PlanView() {
  const data = useGameData();
  const [bases, setBases] = useState<DocumentMeta[] | null>(null);
  const [rosters, setRosters] = useState<DocumentMeta[] | null>(null);
  const [baseId, setBaseId] = useState<string>(() => recall("base") ?? "");
  const [rosterId, setRosterId] = useState<string>(() => recall("roster") ?? "");
  const [base, setBase] = useState<BaseConfig | null>(null);
  const [roster, setRoster] = useState<Roster | null>(null);
  const [horizon, setHorizon] = useState(72);
  const [rotate, setRotate] = useState(true);
  const [swapOut, setSwapOut] = useState(4);
  const [swapIn, setSwapIn] = useState(20);
  const [collectEvery, setCollectEvery] = useState(12);
  const [initialGold, setInitialGold] = useState(0);
  const [weights, setWeights] = useState<Record<string, number>>(() =>
    Object.fromEntries(WEIGHTS.map((w) => [w.key, w.value])),
  );
  const [effort, setEffort] = useState<Effort>("standard");
  const [pins, setPins] = useState<Pin[]>([]);
  const [pinOp, setPinOp] = useState("");
  const [pinRoom, setPinRoom] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.bases.list().then(setBases, (err: unknown) => setError(errorText(err)));
    api.rosters.list().then(setRosters, (err: unknown) => setError(errorText(err)));
  }, []);
  // Fall back to the newest stored document when nothing (or something
  // deleted) was picked before.
  useEffect(() => {
    if (bases && !bases.some((b) => b.id === baseId)) setBaseId(bases[0]?.id ?? "");
  }, [bases, baseId]);
  useEffect(() => {
    if (rosters && !rosters.some((r) => r.id === rosterId)) setRosterId(rosters[0]?.id ?? "");
  }, [rosters, rosterId]);
  useEffect(() => {
    setBase(null);
    setPins([]);
    if (baseId) api.bases.get(baseId).then((d) => setBase(d.base), (err: unknown) => setError(errorText(err)));
  }, [baseId]);
  useEffect(() => {
    setRoster(null);
    setPins([]);
    if (rosterId) api.rosters.get(rosterId).then((d) => setRoster(d.roster), (err: unknown) => setError(errorText(err)));
  }, [rosterId]);

  const capacity = (roomId: string) => {
    const room = base?.rooms.find((r) => r.id === roomId);
    return room ? (data.facilities.get(room.kind)?.phases[room.level - 1]?.max_stationed ?? 0) : 0;
  };
  const rosterOps = Object.keys(roster ?? {})
    .filter((id) => !pins.some((p) => p.operator === id))
    .sort((a, b) => data.name(a).localeCompare(data.name(b)));
  const plan = base ? pinned(base, pins, capacity) : null;

  const solve = async () => {
    if (!base || !plan) return;
    setBusy(true);
    setError(null);
    try {
      const request: Record<string, unknown> = {
        base_id: baseId,
        roster_id: rosterId,
        config: {
          horizon_hours: horizon,
          tick_minutes: 5,
          initial_gold: initialGold,
          collection: collectEvery > 0 ? { kind: "every_hours", hours: collectEvery } : { kind: "continuous" },
        },
        rotation: rotate ? { kind: "mood_threshold", swap_out: swapOut, swap_in: swapIn } : { kind: "none" },
        objective: weights,
        solver: { seed: 1, strategy: "auto", top_k: 5, ...EFFORT[effort].solver },
      };
      if (pins.length > 0) {
        request.initial = plan.initial;
        request.locked = plan.locked;
      }
      const label = name.trim() || `${bases?.find((b) => b.id === baseId)?.name ?? "Base"} · ${new Date().toLocaleString()}`;
      const job = await api.solves.create(request, label);
      remember("base", baseId);
      remember("roster", rosterId);
      go("results", job.id);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const missing = bases?.length === 0 || rosters?.length === 0;

  return (
    <div className="stack">
      {missing && (
        <div className="card">
          <p>
            A plan needs a stored base and a stored roster.
            {bases?.length === 0 && (
              <>
                {" "}
                <a href={href("base")}>Describe your base</a>.
              </>
            )}
            {rosters?.length === 0 && (
              <>
                {" "}
                <a href={href("rosters")}>Import your roster</a>.
              </>
            )}
          </p>
        </div>
      )}
      <div className="card">
        <h2 className="card-title">What to plan</h2>
        <div className="grid-2">
          <label className="field">
            <span>Base</span>
            <select value={baseId} onChange={(e) => setBaseId(e.target.value)}>
              {bases?.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.name ?? "Untitled base"}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Roster</span>
            <select value={rosterId} onChange={(e) => setRosterId(e.target.value)}>
              {rosters?.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name ?? "Untitled roster"}
                </option>
              ))}
            </select>
          </label>
        </div>
        {base && roster && (
          <p className="muted">
            {base.rooms.length} rooms with {fmt(base.rooms.reduce((n, r) => n + capacity(r.id), 0))} slots ·{" "}
            {fmt(Object.keys(roster).length)} operators to choose from
          </p>
        )}
      </div>

      <div className="card">
        <h2 className="card-title">Time and morale</h2>
        <div className="grid-2">
          <fieldset className="segmented" aria-label="Horizon">
            {[24, 72, 168].map((h) => (
              <label key={h} className={horizon === h ? "on" : undefined}>
                <input type="radio" name="horizon" checked={horizon === h} onChange={() => setHorizon(h)} />
                {h === 168 ? "1 week" : `${h} hours`}
              </label>
            ))}
          </fieldset>
          <label className="check">
            <input type="checkbox" checked={rotate} onChange={(e) => setRotate(e.target.checked)} />
            Rotate tired operators through the Dormitories
          </label>
        </div>
        {rotate ? (
          <div className="grid-2">
            <label className="field">
              <span>Send to rest at morale</span>
              <input type="number" min={0} max={24} value={swapOut} onChange={(e) => setSwapOut(Number(e.target.value))} />
            </label>
            <label className="field">
              <span>Back to work at morale</span>
              <input type="number" min={0} max={24} value={swapIn} onChange={(e) => setSwapIn(Number(e.target.value))} />
            </label>
          </div>
        ) : (
          <p className="muted small">
            Without rotation, morale only matters once someone runs out within the horizon, so over 24 hours the
            solver treats morale relief as worthless.
          </p>
        )}
        <details>
          <summary>Collection and starting stock</summary>
          <div className="grid-2">
            <label className="field">
              <span>Collect every (hours, 0 = continuously)</span>
              <input type="number" min={0} value={collectEvery} onChange={(e) => setCollectEvery(Number(e.target.value))} />
            </label>
            <label className="field">
              <span>Pure Gold in the depot at the start</span>
              <input type="number" min={0} value={initialGold} onChange={(e) => setInitialGold(Number(e.target.value))} />
            </label>
          </div>
        </details>
      </div>

      <div className="card">
        <h2 className="card-title">What counts</h2>
        <p className="muted small">
          The score is a weighted sum over the horizon. The defaults price things in LMD where a price exists.
        </p>
        <div className="weights">
          {WEIGHTS.map((w) => (
            <label key={w.key} className="field">
              <span>
                {w.label} <span className="muted small">{w.help}</span>
              </span>
              <input
                type="number"
                step="any"
                value={weights[w.key] ?? 0}
                onChange={(e) => setWeights({ ...weights, [w.key]: Number(e.target.value) })}
              />
            </label>
          ))}
        </div>
        <button
          type="button"
          className="link"
          onClick={() => setWeights(Object.fromEntries(WEIGHTS.map((w) => [w.key, w.value])))}
        >
          Reset to defaults
        </button>
      </div>

      <div className="card">
        <h2 className="card-title">Keep in place</h2>
        <p className="muted small">
          Pinned operators stay where you put them; the solver fills the rest. Leave this empty to let it decide
          everything.
        </p>
        {pins.length > 0 && (
          <ul className="pins">
            {pins.map((p) => {
              const room = base?.rooms.find((r) => r.id === p.room);
              return (
                <li key={p.operator}>
                  {data.name(p.operator)} in {room ? roomName(room.kind) : ""} <code className="muted">{p.room}</code>{" "}
                  <button type="button" className="link" onClick={() => setPins(pins.filter((x) => x !== p))}>
                    Remove
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <div className="row">
          <select value={pinOp} onChange={(e) => setPinOp(e.target.value)} aria-label="Operator to pin" disabled={!roster}>
            <option value="">Operator…</option>
            {rosterOps.map((id) => (
              <option key={id} value={id}>
                {data.name(id)}
              </option>
            ))}
          </select>
          <select value={pinRoom} onChange={(e) => setPinRoom(e.target.value)} aria-label="Room" disabled={!base}>
            <option value="">Room…</option>
            {base?.rooms.map((r) => (
              <option key={r.id} value={r.id}>
                {roomName(r.kind)} {r.id}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="secondary"
            disabled={!pinOp || !pinRoom}
            onClick={() => {
              setPins([...pins, { operator: pinOp, room: pinRoom }]);
              setPinOp("");
            }}
          >
            Pin
          </button>
        </div>
        {plan?.problems.map((p) => (
          <p key={p} className="error">
            {p}
          </p>
        ))}
      </div>

      <div className="card">
        <h2 className="card-title">Solve</h2>
        <fieldset className="segmented" aria-label="Effort">
          {(Object.keys(EFFORT) as Effort[]).map((k) => (
            <label key={k} className={effort === k ? "on" : undefined}>
              <input type="radio" name="effort" checked={effort === k} onChange={() => setEffort(k)} />
              {EFFORT[k].label} <span className="muted small">{EFFORT[k].help}</span>
            </label>
          ))}
        </fieldset>
        <label className="field">
          <span>Name</span>
          <input type="text" value={name} placeholder="Named after the base and time" onChange={(e) => setName(e.target.value)} />
        </label>
        <div className="actions">
          <button
            type="button"
            disabled={busy || !base || !roster || (plan?.problems.length ?? 0) > 0}
            onClick={() => void solve()}
          >
            Solve
          </button>
        </div>
        {error && <p className="error">{error}</p>}
      </div>
    </div>
  );
}
