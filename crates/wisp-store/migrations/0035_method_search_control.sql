ALTER TABLE method_search_runs
ADD COLUMN control_state TEXT NOT NULL DEFAULT 'run';
