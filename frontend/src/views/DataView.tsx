// Game data: where the numbers come from, every operator, and the raw
// request panels for developers.

import { useState } from "react";
import type { GameDataVersion, OperatorSummary } from "../api";
import { IconCode, IconSearch } from "../components/icons";
import { Empty, Meter, PageHeader } from "../components/ui";
import { useGameData } from "../data";
import { Simulator } from "../dev/Simulator";
import { Solver } from "../dev/Solver";
import { fmt, professionName, roomName, short } from "../format";

function VersionCard({ v }: { v: GameDataVersion }) {
  const commitUrl = `https://github.com/${v.repo}/commit/${v.sha}`;
  const s = v.stats;
  const rooms = Object.entries(s.skill_tiers_by_room).sort(([, a], [, b]) => (b ?? 0) - (a ?? 0));
  const most = Math.max(1, ...rooms.map(([, n]) => n ?? 0));
  return (
    <section className="card">
      <header className="card-head">
        <h2 className="card-title">Source</h2>
      </header>
      <div className="card-body stack">
        <dl className="facts">
          <dt>Data</dt>
          <dd>
            {v.source} · <a href={commitUrl}>{v.sha.slice(0, 12)}</a>
            <div className="muted small">{v.repo}</div>
          </dd>
          <dt>Fetched</dt>
          <dd title={v.fetched_at ?? undefined}>{v.fetched_at ? short(v.fetched_at) : "unknown"}</dd>
          <dt>Parser</dt>
          <dd>ak-data {v.parser_version}</dd>
          <dt>Operators</dt>
          <dd>
            {fmt(s.operators)} loaded
            {v.operators_skipped > 0 && <span className="warn"> · {v.operators_skipped} skipped</span>}
          </dd>
        </dl>
        <div className="stack-sm">
          <Meter label="Base skill tiers fully modelled" used={s.mechanics_parsed} limit={s.skill_tiers} />
          <p className="muted small">
            {s.mechanics_coverage_pct.toFixed(1)}% of {fmt(s.skill_tiers)} tiers in {fmt(s.skill_families)} families
            {s.mechanics_partial > 0 && <> · {fmt(s.mechanics_partial)} partial</>}
            {s.mechanics_unparsed > 0 && <span className="warn"> · {fmt(s.mechanics_unparsed)} rejected</span>}
          </p>
        </div>
        <div>
          <div className="section-label">Skill tiers by room</div>
          <ul className="bars">
            {rooms.map(([room, n]) => (
              <li key={room} className="bar-row" title={`${roomName(room as never)}: ${n ?? 0} skill tiers`}>
                <span className="bar-label">{roomName(room as never)}</span>
                <span className="bar-track">
                  <span className="bar-fill" style={{ width: `${((n ?? 0) / most) * 100}%` }} />
                </span>
                <span className="num">{n}</span>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </section>
  );
}

function OperatorTable({ ops }: { ops: OperatorSummary[] }) {
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const shown = ops
    .filter((o) => !q || o.name.toLowerCase().includes(q) || o.id.includes(q))
    .sort((a, b) => b.stars - a.stars || a.name.localeCompare(b.name));
  return (
    <section className="card">
      <header className="card-head">
        <h2 className="card-title">
          Operators <span className="count">{shown.length === ops.length ? fmt(ops.length) : `${fmt(shown.length)} of ${fmt(ops.length)}`}</span>
        </h2>
        <div className="search">
          <IconSearch size={14} />
          <input
            type="search"
            placeholder="Filter by name or id"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Filter operators"
          />
        </div>
      </header>
      {shown.length === 0 ? (
        <Empty title="No operators match">Try part of a name, or an id like char_002_amiya.</Empty>
      ) : (
        <div className="table-scroll">
          <table className="dense">
            <thead>
              <tr>
                <th>Name</th>
                <th className="num">★</th>
                <th>Class</th>
                <th>Subclass</th>
                <th>Faction</th>
                <th className="num">Max level</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((o) => (
                <tr key={o.id}>
                  <td title={o.id}>{o.name}</td>
                  <td className="num">{o.stars}</td>
                  <td>{professionName(o.profession)}</td>
                  <td className="muted">{o.sub_profession}</td>
                  <td className="muted">{[o.nation, o.group, o.team].filter(Boolean).join(" · ")}</td>
                  <td className="num">
                    E{o.max_levels.length - 1} L{o.max_levels.at(-1)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

export function DataView() {
  const data = useGameData();
  const ops = [...data.operators.values()];
  const names = new Map(ops.map((o) => [o.id, o.name]));
  return (
    <>
      <PageHeader title="Game data" description="Where the numbers come from, and every operator the planner knows." />
      <div className="stack">
        <div className="with-sidebar wide">
          <VersionCard v={data.version} />
          <OperatorTable ops={ops} />
        </div>
        <details className="card dev-tools">
          <summary>
            <IconCode size={15} /> Developer tools: raw simulate and solve requests
          </summary>
          <div className="card-body stack">
            <h3 className="section-label">Simulator</h3>
            <Simulator names={names} />
            <h3 className="section-label">Solver</h3>
            <Solver names={names} />
          </div>
        </details>
      </div>
    </>
  );
}
