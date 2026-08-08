UPDATE action_results
SET result = result -> 'result'
WHERE result ? 'result'
  AND jsonb_typeof(result -> 'result') = 'object';
