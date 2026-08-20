-- Immutable marketplace attribution and pricing evidence. Values are nullable
-- for pre-marketplace rows and genuinely unpriced listings.
ALTER TABLE workflow_runs
    ADD COLUMN workflow_author_pubkey BYTEA,
    ADD COLUMN fixed_price_currency VARCHAR(3),
    ADD COLUMN fixed_price_microunits BIGINT,
    ADD CONSTRAINT workflow_runs_fixed_price_pair CHECK (
        (fixed_price_currency IS NULL) = (fixed_price_microunits IS NULL)
    ),
    ADD CONSTRAINT workflow_runs_fixed_price_currency CHECK (
        fixed_price_currency IS NULL OR fixed_price_currency ~ '^[A-Z]{3}$'
    ),
    ADD CONSTRAINT workflow_runs_fixed_price_range CHECK (
        fixed_price_microunits IS NULL OR
        fixed_price_microunits BETWEEN 0 AND 9007199254740991
    );

ALTER TABLE workflow_agent_steps
    ADD COLUMN agent_owner_pubkey BYTEA,
    ADD COLUMN rate_currency VARCHAR(3),
    ADD COLUMN rate_microunits_per_hour BIGINT,
    ADD COLUMN prompt_published_at TIMESTAMPTZ,
    ADD COLUMN completion_event_id TEXT,
    ADD COLUMN terminal_at TIMESTAMPTZ,
    ADD COLUMN duration_ms BIGINT,
    ADD COLUMN outcome VARCHAR(16),
    ADD CONSTRAINT workflow_agent_steps_rate_pair CHECK (
        (rate_currency IS NULL) = (rate_microunits_per_hour IS NULL)
    ),
    ADD CONSTRAINT workflow_agent_steps_rate_currency CHECK (
        rate_currency IS NULL OR rate_currency ~ '^[A-Z]{3}$'
    ),
    ADD CONSTRAINT workflow_agent_steps_rate_range CHECK (
        rate_microunits_per_hour IS NULL OR
        rate_microunits_per_hour BETWEEN 0 AND 9007199254740991
    ),
    ADD CONSTRAINT workflow_agent_steps_duration_nonnegative CHECK (
        duration_ms IS NULL OR duration_ms >= 0
    ),
    ADD CONSTRAINT workflow_agent_steps_outcome CHECK (
        outcome IS NULL OR outcome IN ('completed', 'failed', 'timed_out', 'cancelled')
    );
