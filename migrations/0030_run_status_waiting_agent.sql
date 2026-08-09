-- Distinct run status for agent-assignment suspensions. Runs suspended at an
-- AssignToAgent step were previously parked as 'waiting_approval', which made
-- the two suspend kinds indistinguishable in run listings and UI. PG 12+
-- allows ADD VALUE inside a transaction as long as the value is not used in
-- the same transaction (it is not).
ALTER TYPE run_status ADD VALUE IF NOT EXISTS 'waiting_agent';
