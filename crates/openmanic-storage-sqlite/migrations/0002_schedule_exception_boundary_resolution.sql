-- Preserve the IANA resolver's explicit DST provenance for each overridden
-- occurrence boundary. 0 Exact, 1 FirstValidAfterGap, 2 EarlierInstantInFold.
ALTER TABLE schedule_exception
    ADD COLUMN start_boundary_resolution INTEGER NOT NULL DEFAULT 0
    CHECK(start_boundary_resolution IN (0, 1, 2));
ALTER TABLE schedule_exception
    ADD COLUMN end_boundary_resolution INTEGER NOT NULL DEFAULT 0
    CHECK(end_boundary_resolution IN (0, 1, 2));
