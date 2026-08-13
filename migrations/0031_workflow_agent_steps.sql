-- ── Workflow agent-assignment steps ────────────────────────────────────────
-- Backing table for ActionDef::AssignToAgent: a workflow run suspends and
-- @mentions a channel-member agent, then waits for that agent to reply before
-- resuming. Unlike workflow_approvals there is no secret to hash — the row is
-- keyed by the public event id of the @mention message itself (a later relay
-- hook matches an incoming reply's thread-root/parent back to this id and
-- checks the replying author against agent_pubkey before resuming).

CREATE TYPE agent_step_status AS ENUM ('pending', 'done', 'expired', 'failed');

CREATE TABLE workflow_agent_steps (
    community_id     UUID NOT NULL REFERENCES communities(id),
    prompt_event_id  TEXT NOT NULL,
    workflow_id      UUID NOT NULL,
    run_id           UUID NOT NULL,
    step_id          VARCHAR(64) NOT NULL,
    step_index       INT NOT NULL,
    agent_pubkey     BYTEA NOT NULL,
    status           agent_step_status NOT NULL DEFAULT 'pending',
    output           JSONB,
    expires_at       TIMESTAMPTZ NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at      TIMESTAMPTZ,
    PRIMARY KEY (community_id, prompt_event_id),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    FOREIGN KEY (community_id, run_id)
        REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_workflow_agent_steps_workflow ON workflow_agent_steps (community_id, workflow_id);
CREATE INDEX idx_workflow_agent_steps_run ON workflow_agent_steps (community_id, run_id);
CREATE INDEX idx_workflow_agent_steps_status ON workflow_agent_steps (community_id, status);
