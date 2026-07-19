-- Older builds eagerly created an unused queued job on every analysis request.
-- Analysis starts immediately in the desktop app, so queued analysis rows are stale.
DELETE FROM processing_jobs WHERE job_type = 'analyze' AND status = 'queued';
