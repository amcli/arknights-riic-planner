-- Accounts. Usernames are 3 to 32 ASCII letters, digits, `_` or `-`, unique
-- regardless of case. Case is folded in the "C" collation, which folds A-Z
-- and nothing else, so the fold does not depend on the database's locale.
-- The email is optional and unverified, so it is not unique: nobody can
-- claim someone else's address first, or learn which addresses have
-- accounts.
CREATE TABLE users (
    id            uuid        PRIMARY KEY,
    username      text        NOT NULL CHECK (username ~ '^[A-Za-z0-9_-]{3,32}$'),
    email         text,
    password_hash text        NOT NULL,
    created_at    timestamptz NOT NULL
);

CREATE UNIQUE INDEX users_username_key ON users (lower(username COLLATE "C"));

-- Sign-in sessions. Only a SHA-256 digest of each token is kept, so the
-- table cannot be replayed as live sessions.
CREATE TABLE sessions (
    token_digest bytea       PRIMARY KEY CHECK (length(token_digest) = 32),
    user_id      uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL,
    expires_at   timestamptz NOT NULL
);

CREATE INDEX sessions_by_user ON sessions (user_id);
CREATE INDEX sessions_by_expiry ON sessions (expires_at);
