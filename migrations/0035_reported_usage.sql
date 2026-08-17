-- Agent self-reported usage estimate, written once at the terminal transition
-- and never revised. Sits beside the relay-observed elapsed-time receipt
-- (migration 0033) rather than replacing it: the agent's harness is the only
-- party that sees model usage, so these values are unverified.
ALTER TABLE workflow_agent_steps
    ADD COLUMN usage_harness TEXT,
    ADD COLUMN usage_model TEXT,
    ADD COLUMN usage_input_tokens BIGINT,
    ADD COLUMN usage_output_tokens BIGINT,
    ADD COLUMN usage_cost_microunits BIGINT,
    ADD COLUMN usage_cost_currency VARCHAR(3),
    ADD CONSTRAINT workflow_agent_steps_usage_cost_pair CHECK (
        (usage_cost_microunits IS NULL) = (usage_cost_currency IS NULL)
    ),
    ADD CONSTRAINT workflow_agent_steps_usage_cost_currency CHECK (
        usage_cost_currency IS NULL OR usage_cost_currency ~ '^[A-Z]{3}$'
    ),
    ADD CONSTRAINT workflow_agent_steps_usage_cost_range CHECK (
        usage_cost_microunits IS NULL OR
        usage_cost_microunits BETWEEN 0 AND 9007199254740991
    ),
    ADD CONSTRAINT workflow_agent_steps_usage_input_tokens_range CHECK (
        usage_input_tokens IS NULL OR
        usage_input_tokens BETWEEN 0 AND 9007199254740991
    ),
    ADD CONSTRAINT workflow_agent_steps_usage_output_tokens_range CHECK (
        usage_output_tokens IS NULL OR
        usage_output_tokens BETWEEN 0 AND 9007199254740991
    );
