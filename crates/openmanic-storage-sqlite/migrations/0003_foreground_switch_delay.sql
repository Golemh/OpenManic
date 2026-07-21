ALTER TABLE user_settings
ADD COLUMN foreground_switch_delay_seconds INTEGER NOT NULL DEFAULT 10
CHECK(foreground_switch_delay_seconds BETWEEN 5 AND 20);
