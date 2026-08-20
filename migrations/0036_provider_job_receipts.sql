-- Provider-side ledger for cross-community jobs (BUZZ-10).
--
-- The caller community records its receipt in `workflow_agent_steps`. This
-- table is the symmetric record on the AGENT'S HOME community: which caller
-- communities invoked our listed agent, for how long, and the estimated cost
-- computed from OUR OWN listing rate. Everything here is derivable from the
-- cleartext job-event tags and timestamps the home relay already stores — the
-- caller-encrypted result payload is never read.
--
-- No FK to workflows/workflow_runs: those belong to the caller's community and
-- do not exist here. The (community_id, request_event_id) pair is the identity.
CREATE TABLE provider_agent_jobs (
    community_id            UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    -- kind:43001 request event id — stable per job, our correlation key.
    request_event_id        TEXT NOT NULL,
    -- Cleartext correlation id from the request `request` tag.
    request_id              TEXT NOT NULL,
    -- Our listed agent that did the work.
    agent_pubkey            BYTEA NOT NULL,
    -- Verified owner of that agent (from our local listing), when known.
    agent_owner_pubkey      BYTEA,
    -- The CALLER community that dispatched: identity + routing hint, both read
    -- from the request `relay-pubkey` / `relay` tags (cleartext).
    caller_relay_pubkey     BYTEA NOT NULL,
    caller_relay_url        TEXT NOT NULL,
    -- Our kind:30177 listing snapshot at accept time, and the rate on it.
    listing_event_id        TEXT NOT NULL,
    rate_currency           VARCHAR(3),
    rate_microunits_per_hour BIGINT,
    -- Timing, from event timestamps the home relay observes.
    requested_at            TIMESTAMPTZ NOT NULL,
    terminal_at             TIMESTAMPTZ,
    duration_ms             BIGINT,
    -- Coarse outcome, read from the terminal event KIND only (43004 vs 43003):
    -- 'completed' or 'failed'. The encrypted payload is never inspected.
    outcome                 VARCHAR(16),
    completion_event_id     TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, request_event_id),
    CONSTRAINT provider_agent_jobs_rate_pair CHECK (
        (rate_currency IS NULL) = (rate_microunits_per_hour IS NULL)
    ),
    CONSTRAINT provider_agent_jobs_rate_currency CHECK (
        rate_currency IS NULL OR rate_currency ~ '^[A-Z]{3}$'
    ),
    CONSTRAINT provider_agent_jobs_rate_range CHECK (
        rate_microunits_per_hour IS NULL OR
        rate_microunits_per_hour BETWEEN 0 AND 9007199254740991
    ),
    CONSTRAINT provider_agent_jobs_duration_nonnegative CHECK (
        duration_ms IS NULL OR duration_ms >= 0
    ),
    CONSTRAINT provider_agent_jobs_outcome CHECK (
        outcome IS NULL OR outcome IN ('completed', 'failed')
    )
);

-- Primary listing view: an agent owner's jobs, newest first.
CREATE INDEX idx_provider_agent_jobs_agent
    ON provider_agent_jobs (community_id, agent_pubkey, requested_at DESC);

-- Rollup by caller community.
CREATE INDEX idx_provider_agent_jobs_caller
    ON provider_agent_jobs (community_id, caller_relay_pubkey);
