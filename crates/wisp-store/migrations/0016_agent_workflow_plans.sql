-- Applied column-by-column by Store::apply_agent_workflow_plans so retrying a
-- partially applied migration is safe on SQLite versions without
-- `ADD COLUMN IF NOT EXISTS`.
ALTER TABLE agent_workflows ADD COLUMN frame_id TEXT;
ALTER TABLE agent_workflows ADD COLUMN goal TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_workflows ADD COLUMN mode TEXT NOT NULL DEFAULT 'assisted';
ALTER TABLE agent_workflows ADD COLUMN status TEXT NOT NULL DEFAULT 'draft';
ALTER TABLE agent_workflows ADD COLUMN max_parallel INTEGER NOT NULL DEFAULT 2;
ALTER TABLE agent_workflows ADD COLUMN requires_confirmation INTEGER NOT NULL DEFAULT 1;
ALTER TABLE agent_workflows ADD COLUMN plan_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agent_workflows ADD COLUMN approved_at INTEGER;
ALTER TABLE agent_workflow_steps ADD COLUMN template_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_workflow_steps ADD COLUMN spec_json TEXT NOT NULL DEFAULT '{}';
-- Store::apply_agent_workflow_plans also installs INSERT/UPDATE/DELETE
-- triggers that reject step mutations once the parent leaves draft status.
