# arknights-riic-planner

An accuracy-first optimiser for the Arknights RIIC (base). Rust workspace
backend, React frontend. See [`docs/plan.md`](docs/plan.md) for the full
design; this README tracks what actually exists.

## Status

| Layer | Crate | State |
| --- | --- | --- |
| 1. Data ingestion | `ak-data`, `ak-data-sync` | **Done.** Pinned upstream snapshot, schema drift check, strict two-stage transform. |
| 2. Skill DSL | `ak-data::mechanics` | **Done to 92% / 100%.** Description parser → typed `Mechanics` AST. 589 of 640 tiers fully modelled, 51 partial with named gaps, 0 rejected. |
| 3. Domain model | `ak-domain` | Done for Layers 1–2; `Assignment` and simulation state come with Layer 4. |
| 4. Evaluator / mood sim | `ak-eval` | Placeholder crate. |
| 5. Solver | `ak-solver` | Placeholder crate. |
| 6. Persistence | — | Not started. |
| 7. API | `ak-api` | Read-only game-data endpoints (skills include parsed mechanics). |
| 8. Roster import | — | Not started. |
| 9. Frontend | `frontend/` | Vite + React + TS shell rendering the live game-data endpoints. |

**Ingestion coverage (pinned en_US snapshot):** 374 / 374 operators with
base skills, 640 / 640 skill tiers, 0 skipped under strict mode.

**Parser coverage (same snapshot):**

| Outcome | Tiers | Meaning |
| --- | --- | --- |
| Fully modelled | 589 (92.0%) | Every predicate, counter and effect is a typed AST node the evaluator can act on. |
| Partial | 51 (8.0%) | Parsed, but at least one part is an explicit `Unmodeled` node (e.g. "chance of higher-yield gold orders is increased"). The quantified parts are still usable. |
| Rejected | 0 | The parser could not read the description at all. |

Every parsed magnitude that upstream also exposes through its display-only
`efficiency` sort hint agrees with that hint (over 200 tiers cross-checked
in `crates/ak-data/tests/mechanics.rs`). The coverage numbers are a test
ratchet: they may only go up.

## How Layer 2 works

Upstream ships **no** structured effect data. Each buff has an id, names,
icons, sort keys, a display-only `efficiency` percentage with `targets`, and
the localised description string. The mechanics live only in that string,
but it is highly templated, so the parser is compositional rather than one
rule per sentence:

1. **Templatize** the rich text: `<@cc.vup>+15%</>` → `{V:+15%}`, keywords
   → `{K:Guard}`, glossary terms → `{T:cc.g.bs}` (term ids are stable across
   locales; their display text is not).
2. **Prefix**: strip "When this Operator is assigned to …", harvesting any
   condition it carries ("to the same Trading Post as Texas") or the Workshop
   material filter.
3. **Notes**: harvest stacking ("only the strongest effect of this type") and
   caps from parentheticals, then drop them.
4. **Split** into clauses at `;`, `. `, `, and`, `, but`, ` and `, plain
   commas — only when the following text starts like a new clause, so
   "Caster and Medic Operators'" and "if X, Y" stay whole. A few sentences
   the splitter would mangle are matched whole by *compound* rules first.
5. **Decompose** each clause: leading/trailing condition, leading/trailing
   counter ("for each Blacksteel operator in this Factory", "per Dormitory
   level"), ramps ("+20% in the first hour, then +1%/h up to +25%"), "up to
   N" caps, and "an additional N" clauses that inherit the previous effect's
   kind.
6. **Match** the core phrase against the effect rule table.

The parser never guesses. An unknown phrase rejects the whole tier; a known
but unquantifiable phrase becomes an `Unmodeled` node, so the evaluator can
warn instead of silently applying nothing. The exotic accumulator systems
(Worldly Plight, Perception Information, Felvine, …) are modelled as named
resources with gain / convert / scale-by effects, so they parse honestly and
a later layer can decide how far to simulate them.

Dev loop for improving coverage:

```bash
cargo run -p ak-cli -- mechanics --partial       # rejected + partial tiers
cargo run -p ak-data --example coverage           # rejects grouped by reason
cargo run -p ak-cli -- skill "control_bd_spd[000]"  # one tier, parsed clauses
```

Known modelling gaps (all reported as partial, none silently dropped):
qualitative clue-type biases, "higher-yield gold order" chance, Workshop
byproduct substitution, morale swap / immunity effects, and counters over
things the model does not track yet (orders below the limit, dorm operators
below a morale threshold).

## Layout

```
Cargo.toml              workspace (edition 2024, resolver 3)
data/
  manifest.toml         pinned upstream sources (repo + commit SHA)
  en_US/                fetched files + .sync.json provenance sidecar
crates/
  ak-domain/            newtyped IDs, closed enums, GameData, Mechanics AST
  ak-data/              raw serde mirrors → schema check → strict transform
    src/mechanics/      Layer 2: template, prefix, clause, rules, terms
  ak-data-sync/         fetch pinned SHA, verify digests, validate
  ak-eval/              (placeholder) pure evaluator + mood simulator
  ak-solver/            (placeholder) exhaustive + simulated annealing
  ak-api/               axum: /api/v1/gamedata/*
  ak-cli/               `ak stats | op | skill | find | skipped | mechanics`
frontend/               Vite + React + TypeScript
```

Dependency direction is strictly `ak-domain ← ak-data ← {sync, cli, api}` and
`ak-domain ← ak-eval ← ak-solver`. `ak-eval` must never grow an async or
framework dependency.

## Upstream data

Source: [Kengxxiao/ArknightsGameData_YoStar](https://github.com/Kengxxiao/ArknightsGameData_YoStar),
`en_US`, commit `57010cb5b2af` (2025-11-13, client 31.5.80). That repository
was archived on that date, so this is the final Global snapshot from it; the
CN repository is still live and is listed (disabled) in the manifest with an
identical schema. Operators released to Global after November 2025 are absent
until a successor EN source is chosen. Note that the Layer 2 parser is
written against English descriptions; a CN source would need a second rule
table (term ids and values are shared, prose is not).

Files ingested: `building_data.json`, `character_table.json`,
`handbook_team_table.json` (~16 MB, committed).

### Bumping the pin

1. Edit the `sha` in `data/manifest.toml`.
2. `cargo run -p ak-data-sync -- sync` — fetches, writes `.sync.json`, runs
   the schema drift check and a strict transform.
3. `cargo test --workspace` — update the expectations in
   `crates/ak-data/tests/pinned_data.rs` and the coverage ratchet in
   `crates/ak-data/tests/mechanics.rs` deliberately.
4. Commit the manifest, the data, and the test changes together.

The loader refuses to start if the on-disk sidecar's SHA differs from the
manifest, so a half-done bump fails loudly.

## Running

```bash
cargo test --workspace                     # 79 tests, ~3 s after first build
cargo run -p ak-data-sync -- check         # verify pins, digests, schema, transform
cargo run -p ak-cli -- stats               # counts + parser coverage as JSON
cargo run -p ak-cli -- op char_285_medic2  # one operator, resolved skills
cargo run -p ak-api                        # http://127.0.0.1:8080
```

```bash
cd frontend && npm install && npm run dev  # proxies /api to the backend
```

API endpoints today:

- `GET /healthz`
- `GET /api/v1/gamedata/version` — source, SHA, fetch time, parser version, counts, coverage
- `GET /api/v1/gamedata/operators` — summary list
- `GET /api/v1/gamedata/operators/{id}`
- `GET /api/v1/gamedata/skills/{id}` — includes `mechanics` (null if rejected)

## Toolchain notes

- Rust stable, edition 2024. On Windows without MSVC the GNU host works;
  `ureq` is configured with `native-tls` so no C compiler is needed.
- `[profile.dev.package."*"] opt-level = 2` keeps the 16 MB JSON parse fast in
  test builds.
