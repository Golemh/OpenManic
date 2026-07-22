CREATE TEMP TABLE openmanic_demo_seed_cleanup (
    found INTEGER NOT NULL
) STRICT;

INSERT INTO openmanic_demo_seed_cleanup(found)
SELECT 1
WHERE EXISTS (
    SELECT 1
    FROM tracker_run
    WHERE public_id = X'4F4D52554E00000000000000000000D1'
      AND adapter_version = 'openmanic-demo-v1'
);

DELETE FROM window_title_span
WHERE tracker_run_id IN (
    SELECT id
    FROM tracker_run
    WHERE public_id = X'4F4D52554E00000000000000000000D1'
      AND adapter_version = 'openmanic-demo-v1'
);

DELETE FROM open_activity_checkpoint
WHERE tracker_run_id IN (
    SELECT id
    FROM tracker_run
    WHERE public_id = X'4F4D52554E00000000000000000000D1'
      AND adapter_version = 'openmanic-demo-v1'
);

DELETE FROM activity_interval
WHERE tracker_run_id IN (
    SELECT id
    FROM tracker_run
    WHERE public_id = X'4F4D52554E00000000000000000000D1'
      AND adapter_version = 'openmanic-demo-v1'
);

DELETE FROM tracker_run
WHERE public_id = X'4F4D52554E00000000000000000000D1'
  AND adapter_version = 'openmanic-demo-v1';

DELETE FROM application
WHERE public_id IN (
    X'4F4D41505001000000000000000000D1',
    X'4F4D41505002000000000000000000D1',
    X'4F4D41505003000000000000000000D1',
    X'4F4D41505004000000000000000000D1',
    X'4F4D41505005000000000000000000D1',
    X'4F4D41505006000000000000000000D1',
    X'4F4D41505007000000000000000000D1'
)
AND NOT EXISTS (
    SELECT 1 FROM application_identity WHERE application_id = application.id
)
AND NOT EXISTS (
    SELECT 1 FROM activity_interval WHERE application_id = application.id
)
AND NOT EXISTS (
    SELECT 1 FROM open_activity_checkpoint WHERE application_id = application.id
)
AND NOT EXISTS (
    SELECT 1 FROM window_title_span WHERE application_id = application.id
);

UPDATE store_metadata
SET data_revision = data_revision + 1
WHERE singleton_id = 1
  AND EXISTS (SELECT 1 FROM openmanic_demo_seed_cleanup);

DROP TABLE openmanic_demo_seed_cleanup;
