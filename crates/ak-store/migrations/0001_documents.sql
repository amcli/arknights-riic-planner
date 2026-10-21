-- Rosters, bases and solve jobs: one JSON document each. Bodies are never
-- decomposed into columns, and old ones are migrated on read
-- (`ak_store::migrate`), never in place.
CREATE TABLE documents (
    collection     text        NOT NULL CHECK (collection IN ('rosters', 'bases', 'solves')),
    id             uuid        NOT NULL,
    schema_version integer     NOT NULL CHECK (schema_version > 0),
    created_at     timestamptz NOT NULL,
    updated_at     timestamptz NOT NULL,
    name           text,
    body           jsonb       NOT NULL,
    PRIMARY KEY (collection, id)
);

-- Listings are newest first.
CREATE INDEX documents_newest_first ON documents (collection, created_at DESC, id);
