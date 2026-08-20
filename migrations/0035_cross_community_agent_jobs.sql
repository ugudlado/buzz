-- Cross-community job events (43001–43006) carry NIP-44 ciphertext and must
-- remain unsearchable. Preserve the installation's current FTS policy while
-- adding the exclusion; PostgreSQL cannot alter a generated expression in place.
ALTER TABLE workflow_agent_steps
    ADD COLUMN request_id TEXT,
    ADD COLUMN origin_relay_pubkey BYTEA,
    ADD COLUMN agent_relay_pubkey BYTEA,
    ADD COLUMN agent_relay_url TEXT,
    ADD COLUMN listing_event_id TEXT,
    ADD COLUMN delivered_at TIMESTAMPTZ,
    ADD COLUMN delivery_attempts INT NOT NULL DEFAULT 0,
    ADD COLUMN next_delivery_at TIMESTAMPTZ,
    ADD COLUMN last_delivery_error TEXT,
    ADD CONSTRAINT workflow_agent_steps_remote_pair CHECK (
        (request_id IS NULL AND origin_relay_pubkey IS NULL AND
         agent_relay_pubkey IS NULL AND agent_relay_url IS NULL AND
         listing_event_id IS NULL)
        OR
        (request_id IS NOT NULL AND origin_relay_pubkey IS NOT NULL AND
         agent_relay_pubkey IS NOT NULL AND agent_relay_url IS NOT NULL AND
         listing_event_id IS NOT NULL)
    ),
    ADD CONSTRAINT workflow_agent_steps_delivery_attempts_nonnegative CHECK (
        delivery_attempts >= 0
    );

CREATE UNIQUE INDEX idx_workflow_agent_steps_remote_request
    ON workflow_agent_steps (community_id, request_id)
    WHERE request_id IS NOT NULL;
CREATE INDEX idx_workflow_agent_steps_remote_delivery
    ON workflow_agent_steps (next_delivery_at)
    WHERE status = 'pending' AND request_id IS NOT NULL AND delivered_at IS NULL;

DO $$
DECLARE
    existing_expression TEXT;
BEGIN
    SELECT pg_get_expr(d.adbin, d.adrelid)
      INTO existing_expression
      FROM pg_attrdef d
      JOIN pg_attribute a
        ON a.attrelid = d.adrelid
       AND a.attnum = d.adnum
     WHERE d.adrelid = 'events'::regclass
       AND a.attname = 'search_tsv';

    IF existing_expression IS NULL THEN
        RAISE EXCEPTION 'events.search_tsv generated expression not found';
    END IF;

    ALTER TABLE events DROP COLUMN search_tsv;
    EXECUTE format(
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind BETWEEN 43001 AND 43006 THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
