// Base: describe the rooms of a base (kind, level and settings), see it on
// the floor plan, and store it. The server validates on save; the budgets
// shown here use the same data, so they agree with it.

import { useEffect, useState } from "react";
import { api, type BaseConfig, type DocumentMeta, type Profession, type Room, type RoomType } from "../api";
import { BaseMap, toneOf } from "../components/BaseMap";
import { IconCheck, IconCopy, IconPlus, IconSave, IconTrash, IconX } from "../components/icons";
import { Callout, Meter, PageHeader } from "../components/ui";
import { useGameData, type GameData } from "../data";
import { ago, byFormulaId, errorText, formulaLabel, PROFESSIONS, roomName, when } from "../format";
import { EXAMPLE_BASE } from "../presets";
import { recall, remember } from "../storage";

/** Room kinds a base can staff, in display order. */
const KINDS: RoomType[] = ["CONTROL", "TRADING", "MANUFACTURE", "POWER", "DORMITORY", "MEETING", "HIRE", "WORKSHOP", "TRAINING"];

const ID_PREFIX: Record<string, string> = {
  CONTROL: "cc",
  POWER: "p",
  MANUFACTURE: "f",
  TRADING: "t",
  DORMITORY: "d",
  WORKSHOP: "ws",
  HIRE: "office",
  TRAINING: "training",
  MEETING: "reception",
};

const EMPTY_BASE: BaseConfig = { rooms: [{ id: "cc", kind: "CONTROL", level: 5 }] };

function newId(base: BaseConfig, kind: RoomType, max: number | null): string {
  const prefix = ID_PREFIX[kind] ?? kind.toLowerCase();
  const taken = new Set(base.rooms.map((r) => r.id));
  if (max === 1 && !taken.has(prefix)) return prefix;
  for (let n = 1; ; n++) if (!taken.has(`${prefix}${n}`)) return `${prefix}${n}`;
}

function defaults(kind: RoomType, data: GameData): Room["settings"] {
  switch (kind) {
    case "MANUFACTURE":
      return { formula: "4" };
    case "TRADING":
      return { strategy: "gold" };
    case "DORMITORY":
      return { ambience: data.constants.comfort_limit };
    default:
      return undefined;
  }
}

interface Budget {
  label: string;
  used: number;
  limit: number;
}

function budgets(base: BaseConfig, data: GameData): Budget[] {
  let supply = 0;
  let demand = 0;
  const byCategory = new Map<string, number>();
  for (const r of base.rooms) {
    const f = data.facilities.get(r.kind);
    const e = f?.phases[r.level - 1]?.electricity ?? 0;
    if (e >= 0) supply += e;
    else demand -= e;
    if (f) byCategory.set(f.category, (byCategory.get(f.category) ?? 0) + 1);
  }
  const slots = (cat: string) => data.layout.slots.filter((s) => s.category === cat).length;
  return [
    { label: "Power drawn", used: demand, limit: supply },
    { label: "Production slots", used: byCategory.get("OUTPUT") ?? 0, limit: slots("OUTPUT") },
    { label: "Dormitories", used: byCategory.get("CUSTOM") ?? 0, limit: slots("CUSTOM") },
    { label: "Function rooms", used: byCategory.get("FUNCTION") ?? 0, limit: slots("FUNCTION") },
  ];
}

function RoomRow({
  room,
  onChange,
  onRemove,
}: {
  room: Room;
  onChange: (r: Room) => void;
  onRemove?: () => void;
}) {
  const data = useGameData();
  const f = data.facilities.get(room.kind);
  const levels = f?.phases.map((p) => p.level) ?? [1];
  const s = room.settings ?? {};
  const set = (settings: Room["settings"]) => onChange({ ...room, settings });
  const formulas = [...data.formulas.values()]
    .filter((fm) => fm.require_rooms.every((req) => req.room_type !== "MANUFACTURE" || req.level <= room.level))
    .sort(byFormulaId);

  const setLevel = (level: number) => {
    const next: Room = { ...room, level };
    const formula = s.formula ? data.formulas.get(s.formula) : undefined;
    const needs = formula?.require_rooms.find((q) => q.room_type === "MANUFACTURE")?.level ?? 1;
    if (room.kind === "MANUFACTURE" && needs > level) next.settings = { ...s, formula: "4" };
    onChange(next);
  };

  return (
    <li className="room-row">
      <span className={`swatch ${toneOf(room.kind, data.facilities)}`} aria-hidden="true" />
      <span className="room-name" title={`${roomName(room.kind)} (${room.id})`}>
        {roomName(room.kind)} <span className="id">{room.id}</span>
      </span>
      <select value={room.level} aria-label={`Level of ${room.id}`} onChange={(e) => setLevel(Number(e.target.value))}>
        {levels.map((l) => (
          <option key={l} value={l}>
            Level {l}
          </option>
        ))}
      </select>
      <div className="room-settings">
        {room.kind === "MANUFACTURE" && (
          <select
            value={s.formula ?? ""}
            aria-label={`Formula of ${room.id}`}
            onChange={(e) => set({ ...s, formula: e.target.value || null })}
          >
            <option value="">Idle</option>
            {formulas.map((fm) => (
              <option key={fm.id} value={fm.id}>
                {formulaLabel(fm, data.formulas.values())}
              </option>
            ))}
          </select>
        )}
        {room.kind === "TRADING" && (
          <select
            value={s.strategy ?? "gold"}
            aria-label={`Orders of ${room.id}`}
            onChange={(e) => set({ ...s, strategy: e.target.value as "gold" | "originium_shard" })}
          >
            <option value="gold">Pure Gold → LMD</option>
            <option value="originium_shard">Originium Shard → Orundum</option>
          </select>
        )}
        {room.kind === "DORMITORY" && (
          <label className="inline">
            <span>Ambience</span>
            <input
              type="number"
              min={0}
              max={data.constants.comfort_limit}
              step={100}
              value={s.ambience ?? 0}
              onChange={(e) =>
                set({ ...s, ambience: Math.max(0, Math.min(data.constants.comfort_limit, Number(e.target.value) || 0)) })
              }
            />
          </label>
        )}
        {room.kind === "TRAINING" && (
          <>
            <select
              value={s.training?.profession ?? ""}
              aria-label={`Trainee class for ${room.id}`}
              onChange={(e) =>
                set({
                  ...s,
                  training: e.target.value
                    ? { profession: e.target.value as Profession, spec_level: s.training?.spec_level ?? 3 }
                    : null,
                })
              }
            >
              <option value="">Idle</option>
              {PROFESSIONS.map((p) => (
                <option key={p.id} value={p.id}>
                  Training a {p.name}
                </option>
              ))}
            </select>
            {s.training && (
              <select
                className="spec"
                value={s.training.spec_level}
                aria-label={`Specialization level for ${room.id}`}
                onChange={(e) =>
                  s.training && set({ ...s, training: { ...s.training, spec_level: Number(e.target.value) } })
                }
              >
                {[1, 2, 3].map((l) => (
                  <option key={l} value={l}>
                    to S{l}
                  </option>
                ))}
              </select>
            )}
          </>
        )}
      </div>
      {onRemove ? (
        <button type="button" className="ghost icon sm" onClick={onRemove} aria-label={`Remove ${room.id}`} title="Remove room">
          <IconX size={14} />
        </button>
      ) : (
        <span />
      )}
    </li>
  );
}

function Editor({
  doc,
  onSaved,
  onDeleted,
  active,
  onUse,
}: {
  doc: { id?: string; name: string; base: BaseConfig };
  onSaved: (id: string) => void;
  onDeleted: () => void;
  active: boolean;
  onUse: () => void;
}) {
  const data = useGameData();
  const [base, setBase] = useState(doc.base);
  const [name, setName] = useState(doc.name);
  const [adding, setAdding] = useState<RoomType>("MANUFACTURE");
  const [message, setMessage] = useState<{ error?: string; ok?: string }>({});
  const [busy, setBusy] = useState(false);
  const dirty = name !== doc.name || JSON.stringify(base) !== JSON.stringify(doc.base);

  const counts = new Map<RoomType, number>();
  for (const r of base.rooms) counts.set(r.kind, (counts.get(r.kind) ?? 0) + 1);
  const addable = KINDS.filter((k) => {
    const max = data.facilities.get(k)?.max_count ?? null;
    return max === null || (counts.get(k) ?? 0) < max;
  });
  const kind = addable.includes(adding) ? adding : addable[0];

  const add = () => {
    if (!kind) return;
    const f = data.facilities.get(kind);
    const room: Room = { id: newId(base, kind, f?.max_count ?? null), kind, level: f?.phases.length ?? 1 };
    const settings = defaults(kind, data);
    if (settings) room.settings = settings;
    setBase({ rooms: [...base.rooms, room] });
    setMessage({});
  };

  const save = async (asNew: boolean) => {
    setBusy(true);
    setMessage({});
    try {
      const label = name.trim() || undefined;
      const meta =
        doc.id && !asNew ? await api.bases.update(doc.id, base, label) : await api.bases.create(base, label);
      setMessage({ ok: "Saved." });
      onSaved(meta.id);
    } catch (err) {
      setMessage({ error: errorText(err) });
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!doc.id || !window.confirm(`Delete the base "${doc.name || doc.id}"? This cannot be undone.`)) return;
    try {
      await api.bases.remove(doc.id);
      onDeleted();
    } catch (err) {
      setMessage({ error: errorText(err) });
    }
  };

  const ordered = [...base.rooms].sort((a, b) => KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind));

  return (
    <section className="card">
      <header className="card-head editor-head">
        <div className="title-input">
          <input
            type="text"
            value={name}
            placeholder="Name this base"
            aria-label="Name"
            onChange={(e) => setName(e.target.value)}
          />
          {active && <span className="badge accent dot">Used for planning</span>}
          {!doc.id ? (
            <span className="badge warn dot">Not saved yet</span>
          ) : dirty ? (
            <span className="badge warn dot">Unsaved changes</span>
          ) : (
            message.ok && <span className="badge good dot">Saved</span>
          )}
        </div>
        <div className="row">
          {doc.id && !active && (
            <button type="button" className="secondary sm" onClick={onUse}>
              <IconCheck size={14} /> Use for planning
            </button>
          )}
          {doc.id && (
            <button type="button" className="ghost danger sm" onClick={() => void remove()}>
              <IconTrash size={14} /> Delete
            </button>
          )}
          {doc.id && (
            <button type="button" className="secondary sm" disabled={busy} onClick={() => void save(true)}>
              <IconCopy size={14} /> Save as new
            </button>
          )}
          <button type="button" className="sm" disabled={busy} onClick={() => void save(false)}>
            <IconSave size={14} /> {doc.id ? "Save changes" : "Save base"}
          </button>
        </div>
      </header>
      <div className="meters">
        {budgets(base, data).map((b) => (
          <Meter key={b.label} label={b.label} used={b.used} limit={b.limit} />
        ))}
      </div>
      {message.error && (
        <div className="card-body">
          <Callout tone="error">{message.error}</Callout>
        </div>
      )}
      <div className="map-panel">
        <BaseMap base={base} label="The base's rooms on the in-game floor plan" />
      </div>
      <div className="card-body">
        <div className="section-label">
          Rooms <span className="count">{base.rooms.length}</span>
        </div>
        <ul className="room-grid">
          {ordered.map((room) => (
            <RoomRow
              key={room.id}
              room={room}
              onChange={(next) => setBase({ rooms: base.rooms.map((r) => (r.id === room.id ? next : r)) })}
              onRemove={
                room.kind === "CONTROL"
                  ? undefined
                  : () => setBase({ rooms: base.rooms.filter((r) => r.id !== room.id) })
              }
            />
          ))}
        </ul>
        <div className="add-room">
          <select
            value={kind ?? ""}
            aria-label="Room kind to add"
            onChange={(e) => setAdding(e.target.value as RoomType)}
            disabled={!kind}
          >
            {addable.map((k) => (
              <option key={k} value={k}>
                {roomName(k)}
              </option>
            ))}
          </select>
          <button type="button" className="secondary sm" onClick={add} disabled={!kind}>
            <IconPlus size={14} /> Add room
          </button>
          {!kind && <span className="muted small">Every kind of room is at its limit.</span>}
        </div>
      </div>
    </section>
  );
}

export function BaseView() {
  const [list, setList] = useState<DocumentMeta[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState<string | null>(() => recall("base"));
  const [editing, setEditing] = useState<{ key: string; id?: string; name: string; base: BaseConfig } | null>(null);

  const refresh = () => api.bases.list().then(setList, (err: unknown) => setError(errorText(err)));
  useEffect(() => {
    void refresh();
  }, []);

  const open = (id: string) => {
    setError(null);
    api.bases.get(id).then(
      (doc) => setEditing({ key: `${doc.id}:${doc.updated_at}`, id: doc.id, name: doc.name ?? "", base: doc.base }),
      (err: unknown) => setError(errorText(err)),
    );
  };
  useEffect(() => {
    if (editing || !list) return;
    const first = list.find((b) => b.id === active) ?? list[0];
    if (first) open(first.id);
    else setEditing({ key: "example", name: "2-4-3 example", base: EXAMPLE_BASE });
    // Open something once the list has loaded.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list]);

  const use = (id: string | null) => {
    remember("base", id);
    setActive(id);
  };

  return (
    <>
      <PageHeader title="Base" description="Your rooms: levels, what each Factory makes, what each Trading Post sells." />
      <div className="with-sidebar narrow">
        <section className="card">
          <header className="card-head">
            <h2 className="card-title">
              Saved bases {list && list.length > 0 && <span className="count">{list.length}</span>}
            </h2>
          </header>
          {error && (
            <div className="card-body">
              <Callout tone="error">{error}</Callout>
            </div>
          )}
          {list && list.length === 0 && <p className="doc-empty">None yet. Start from the example and save it.</p>}
          {list && list.length > 0 && (
            <ul className="doc-list">
              {list.map((b) => (
                <li key={b.id}>
                  <button
                    type="button"
                    className={b.id === editing?.id ? "doc on" : "doc"}
                    onClick={() => open(b.id)}
                    aria-current={b.id === editing?.id}
                  >
                    <span className="doc-name">
                      <span>{b.name ?? "Untitled base"}</span>
                      {b.id === active && <span className="badge accent">In use</span>}
                    </span>
                    <span className="doc-meta" title={when(b.updated_at)}>
                      Updated {ago(b.updated_at)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="card-foot new-buttons">
            <button
              type="button"
              className="secondary sm full"
              onClick={() => setEditing({ key: `example-${Date.now()}`, name: "2-4-3 example", base: EXAMPLE_BASE })}
            >
              <IconPlus size={14} /> New from the 2-4-3 example
            </button>
            <button
              type="button"
              className="ghost sm full"
              onClick={() => setEditing({ key: `empty-${Date.now()}`, name: "", base: EMPTY_BASE })}
            >
              <IconPlus size={14} /> New empty base
            </button>
          </div>
        </section>
        <div className="min0">
          {editing ? (
            <Editor
              key={editing.key}
              doc={editing}
              active={editing.id !== undefined && editing.id === active}
              onUse={() => editing.id && use(editing.id)}
              onSaved={(id) => {
                void refresh();
                if (!active) use(id);
                open(id);
              }}
              onDeleted={() => {
                if (active === editing.id) use(null);
                setEditing(null);
                void refresh();
              }}
            />
          ) : (
            <p className="boot">
              <span className="spinner" /> Loading…
            </p>
          )}
        </div>
      </div>
    </>
  );
}
