-- Every document belongs to an account, and goes when the account does.
-- Documents stored before accounts existed have no owner; rather than
-- delete or guess, this refuses to run while any exist.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM documents) THEN
        RAISE EXCEPTION 'documents stored before accounts existed have no owner: delete them, or give them one, before migrating';
    END IF;
END
$$;

ALTER TABLE documents
    ADD COLUMN owner_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE;

-- Listings are one owner's, newest first.
DROP INDEX documents_newest_first;
CREATE INDEX documents_by_owner ON documents (owner_id, collection, created_at DESC, id);
